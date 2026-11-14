// WP P1WB2: the Installer page (the Installer mode's "Limits and power-on" screen of the mode
// manifest; PRO1 S1, S2, S4, S5). Hardware settings apply at once: no Save bar here, every
// change is a write, journalled like any other.
//
// - The lock (S4): whose it is and whether it holds, Unlock and Lock again, what it covers.
//   The web keeps no installer PIN of its own: a lock comes from the hub that serves the DSPi.
// - Limits (S1, P1LM): per output the highest gain, the limiter's ceiling, the high-pass it
//   must keep and the delay range; the highest PEQ boost and master volume; the installer's
//   note and contact. Each change applies the profile at once and shows what it held. This
//   page keeps the profile per device and gives it to the core whenever the device opens.
// - Outputs without a limiter (S2) and Protect all outputs.
// - At power-on (S5): the startup preset, where the master volume and the output
//   configuration are kept, the DAC's hardware mute, the S/PDIF and ADAT line, and "Test
//   power-on", which loads the startup preset as the device would.

import { useEffect, useRef, useState } from "preact/hooks";
import type { DacMuteRecord, DspState, LimitsApplyRecord, LockStateRecord, ModeManifest } from "../../core/protocol";
import { safeLocalStorage } from "../../fleet/local";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { noticeOf, noticeRow } from "../../ui/texts";
import { outputRows } from "../main/model";
import { download } from "../snapshots/model";
import {
  OUTPUT_FIELDS,
  STORAGE_CHOICES,
  limitText,
  limitValue,
  lockLine,
  parseProfile,
  powerOnSlot,
  profileDocument,
  saveProfile,
  startupBytes,
  withField,
  withOutputLimit,
  type LimitsProfile,
} from "./model";
import { UnlockDialog, scopeLine } from "./Unlock";

const PRESET_SLOTS = 10;

export function InstallerScreen(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const snap = st.snapshot;
  if (!snap) return null;
  const lock = st.lock ?? null;
  // A PIN set and this console not unlocked: the installer's parts stay as they are.
  const gated = !!lock && !lock.installerOpen;
  const ro = session.readOnlyReason;
  return (
    <div class="installer-screen">
      <LockPanel session={session} lock={lock} />
      <LimitsPanel session={session} s={snap.state} serial={snap.device.serial || session.deviceId} disabled={!!ro || gated} labels={snap.outputLabels} />
      <PowerOnPanel session={session} s={snap.state} disabled={!!ro} ask={props.ask} unsaved={props.unsaved} />
    </div>
  );
}

// ---------------------------------------------------------------- the lock

function LockPanel(props: { session: Session; lock: LockStateRecord | null }) {
  const { session, lock } = props;
  const [asking, setAsking] = useState(false);
  const [manifest, setManifest] = useState<ModeManifest | null>(null);
  useEffect(() => {
    void session.core.call("modeManifest", undefined).then(setManifest, () => setManifest(null));
    void session.refreshLock();
  }, [session]);
  const pinSet = !!lock?.pinSet;
  const covers = new Set(lock?.covers ?? []);
  const cover = (id: string, on: boolean) => {
    const next = (manifest?.screens ?? []).map((x) => x.id).filter((x) => (x === id ? on : covers.has(x)));
    void session.core.call("installerLockCover", { pages: next }).then(
      () => session.refreshLock(),
      (e) => session.fail(e),
    );
  };
  return (
    <section class="card installer-lock" aria-label="Installer lock">
      <h3>Installer lock</h3>
      <p class="lock-state">{lockLine(lock)}</p>
      {pinSet ? <p class="note lock-scope">{scopeLine(lock)}</p> : null}
      <p class="note">
        The web console keeps no installer PIN of its own. A DSPi served by a hub has the hub's lock: an Admin sets its PIN on the hub, and Unlock here
        asks the hub.
      </p>
      {pinSet ? (
        <div class="installer-buttons">
          {lock!.unlocked ? (
            <button type="button" onClick={() => void session.relock()}>
              Lock again
            </button>
          ) : (
            <button type="button" onClick={() => setAsking(true)}>
              Unlock…
            </button>
          )}
        </div>
      ) : null}
      {pinSet && manifest ? (
        <fieldset class="lock-covers" disabled={!lock!.installerOpen}>
          <legend>What the lock covers</legend>
          {manifest.screens.map((x) => (
            <label key={x.id}>
              <input type="checkbox" checked={covers.has(x.id)} onChange={(e) => cover(x.id, (e.target as HTMLInputElement).checked)} /> {x.title}
            </label>
          ))}
        </fieldset>
      ) : null}
      {asking ? <UnlockDialog session={session} lock={lock} onDone={() => setAsking(false)} /> : null}
    </section>
  );
}

// ---------------------------------------------------------------- limits

function LimitsPanel(props: { session: Session; s: DspState; serial: string; disabled: boolean; labels?: Parameters<typeof outputRows>[1] }) {
  const { session, s, serial, disabled } = props;
  const [profile, setProfile] = useState<LimitsProfile>({});
  const [report, setReport] = useState<{ title: string; lines: string[]; tone: "info" | "warn" | "error" } | null>(null);
  const [status, setStatus] = useState("");
  const [bare, setBare] = useState<number[]>([]);
  const file = useRef<HTMLInputElement>(null);
  const storage = safeLocalStorage();

  // The profile the core holds for this device, and its standing notice.
  useEffect(() => {
    void session.core.call("limitsProfile", undefined).then(
      (r) => setProfile(parseProfile(r.text) ?? {}),
      () => setProfile({}),
    );
    void session.core.call("limitsStatus", undefined).then(
      (r) => {
        const n = r.state === "fell_back" || r.state === "unreadable" ? noticeRow(r.noticeKey) : null;
        setStatus(n ? `${n.title}: ${n.reason}${r.detail ? ` (${r.detail})` : ""}` : "");
      },
      () => setStatus(""),
    );
  }, [session]);

  // S2: the outputs louder than -10 dB without a limiter, again after every change.
  useEffect(() => {
    void session.core.call("outputsWithoutLimiter", undefined).then(setBare, () => setBare([]));
  }, [session, s]);

  const apply = async (next: LimitsProfile) => {
    const before = profile;
    setProfile(next);
    try {
      const text = profileDocument({ ...next, serial: next.serial || serial });
      const r: LimitsApplyRecord = await session.core.call("limitsApply", { text });
      saveProfile(storage, serial, text);
      setReport(r.held.length ? { title: "Held to the new limits", lines: r.held, tone: "warn" } : null);
      await session.refresh();
    } catch (e) {
      setProfile(before);
      const n = noticeOf(e);
      setReport({ title: n ? n.title : "Not applied", lines: [n ? `${n.reason} ${n.detail}`.trim() : String((e as Error).message ?? e)], tone: "error" });
    }
  };

  const clear = async () => {
    try {
      await session.core.call("limitsClear", undefined);
      saveProfile(storage, serial, "");
      setProfile({});
      setReport(null);
    } catch (e) {
      session.fail(e);
    }
  };

  const load = async (input: HTMLInputElement) => {
    const f = input.files?.[0];
    input.value = "";
    if (!f) return;
    const p = parseProfile(await f.text());
    if (!p) {
      setReport({ title: "Not a limits profile", lines: [`${f.name} holds no limits profile.`], tone: "error" });
      return;
    }
    await apply({ ...p, serial });
  };

  const outputs = Array.from({ length: s.numOutputChannels }, (_, o) => o);
  const names = new Map(outputRows({ ...s, outputEnabled: s.outputEnabled.map(() => true) }, props.labels).map((r) => [r.index, `${r.descriptor} ${r.name}`]));
  const forOutput = (o: number) => profile.outputs?.find((x) => x.output === o);
  const field = (label: string, value: number | undefined, min: number, max: number, step: number, onSet: (v: number | undefined) => void) => (
    <label class="limit-field">
      <span>{label}</span>
      <input
        type="number"
        inputMode="decimal"
        min={min}
        max={max}
        step={step}
        placeholder="none"
        value={limitText(value)}
        disabled={disabled}
        onChange={(e) => onSet(limitValue((e.target as HTMLInputElement).value, min, max))}
      />
    </label>
  );

  return (
    <section class="card installer-limits" aria-label="Limits">
      <h3>Limits</h3>
      <p class="note">
        A change applies at once and holds the DSPi to it. A field left empty is not limited. A limit the firmware does not know can be crossed from a
        control surface, the UART or another console that does not have this profile.
      </p>
      {status ? (
        <p class="error" role="alert">
          {status}
        </p>
      ) : null}
      {bare.length ? (
        <div class="limits-warning" role="status" aria-label="Outputs without a limiter">
          <span>
            {bare.length === 1 ? "An output above -10 dB gain has no limiter: " : "Outputs above -10 dB gain have no limiter: "}
            {bare.map((o) => names.get(o) ?? `OUT${o + 1}`).join(", ")}
          </span>
          <button type="button" disabled={!!session.readOnlyReason} onClick={() => void session.change("protectAllOutputs", undefined)}>
            Protect all outputs
          </button>
        </div>
      ) : null}
      <div class="limits-device">
        {field("Highest PEQ boost (dB)", profile.maxPeqBoostDb, 0, 20, 0.5, (v) => void apply(withField(profile, "maxPeqBoostDb", v)))}
        {field("Highest master volume (dB)", profile.masterVolumeMaxDb, -127, 0, 0.5, (v) => void apply(withField(profile, "masterVolumeMaxDb", v)))}
      </div>
      <div class="limits-outputs">
        {outputs.map((o) => (
          <fieldset key={o} class="limits-output" disabled={disabled}>
            <legend>{names.get(o) ?? `OUT${o + 1}`}</legend>
            {OUTPUT_FIELDS.map((f) =>
              field(`${f.label} (${f.unit})`, forOutput(o)?.[f.key], f.min, f.max, f.step, (v) => void apply(withOutputLimit(profile, o, f.key, v))),
            )}
          </fieldset>
        ))}
      </div>
      <div class="limits-hand-over">
        <label class="limit-text">
          <span>Note for whoever looks after the system</span>
          <textarea rows={2} value={profile.note ?? ""} disabled={disabled} onChange={(e) => void apply(withField(profile, "note", (e.target as HTMLTextAreaElement).value))} />
        </label>
        <label class="limit-text">
          <span>How to reach the installer</span>
          <input type="text" value={profile.contact ?? ""} disabled={disabled} onChange={(e) => void apply(withField(profile, "contact", (e.target as HTMLInputElement).value))} />
        </label>
      </div>
      {report ? (
        <div class={`limits-report ${report.tone}`} role={report.tone === "error" ? "alert" : "status"} aria-label="Limits report">
          <strong>{report.title}</strong>
          <ul>
            {report.lines.map((l, i) => (
              <li key={i}>{l}</li>
            ))}
          </ul>
        </div>
      ) : null}
      <div class="installer-buttons">
        <button type="button" onClick={() => download(profileDocument({ ...profile, serial }), `limits-${serial || "dspi"}.dspipreset`)}>
          Download Profile
        </button>
        <button type="button" disabled={disabled} onClick={() => file.current?.click()}>
          Load Profile…
        </button>
        <input ref={file} type="file" hidden accept=".dspipreset,.json,application/json" aria-label="Limits profile to load" onChange={(e) => void load(e.target as HTMLInputElement)} />
        <button type="button" class="danger" disabled={disabled} onClick={() => void clear()}>
          Clear Profile
        </button>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------- power-on

function PowerOnPanel(props: { session: Session; s: DspState; disabled: boolean; ask: ScreenProps["ask"]; unsaved: ScreenProps["unsaved"] }) {
  const { session, s, disabled } = props;
  const [dac, setDac] = useState<DacMuteRecord | null>(null);
  const [note, setNote] = useState("");
  useEffect(() => {
    void session.core.call("dacMuteFetch", undefined).then(setDac, () => setDac(null));
  }, [session]);
  const lastUsed = s.presetStartupMode === 1;
  const slot = s.presetDefaultSlot ?? 0;
  const names = s.presetNames ?? [];
  const slotName = (i: number) => (names[i]?.trim() ? `${i + 1}: ${names[i]}` : `Preset ${i + 1}`);
  const setDacField = async (patch: Partial<DacMuteRecord>) => {
    if (!dac) return;
    const next = { ...dac, ...patch };
    setDac(next);
    try {
      await session.core.call("dacMuteSet", { enabled: next.enabled, activeLow: next.activeLow, pin: next.pin, holdMs: next.holdMs, releaseMs: next.releaseMs });
      // The firmware applies it in its main loop: read it back a moment later.
      await new Promise((r) => setTimeout(r, 200));
      setDac(await session.core.call("dacMuteFetch", undefined));
    } catch (e) {
      session.fail(e);
      void session.core.call("dacMuteFetch", undefined).then(setDac, () => undefined);
    }
  };
  const test = async () => {
    const target = powerOnSlot(s.presetStartupMode, slot, s.activePresetSlot);
    const a = await props.unsaved();
    if (a === "cancel") return;
    if (a === "save" && !(await session.savePreset(s.activePresetSlot))) return;
    if (await session.write("preset.load", "trigger", [target])) setNote(`Loaded ${slotName(target)}, as at power-on.`);
  };
  const ms = (label: string, value: number, onSet: (v: number) => void) => (
    <label class="limit-field">
      <span>{label}</span>
      <input
        type="number"
        min={0}
        max={1000}
        step={1}
        value={value}
        disabled={disabled}
        onChange={(e) => onSet(Math.max(0, Math.min(1000, Math.round(Number((e.target as HTMLInputElement).value) || 0))))}
      />
    </label>
  );
  return (
    <section class="card installer-power" aria-label="At power-on">
      <h3>At power-on</h3>
      <p class="note">Changes apply at once.</p>
      <div class="power-rows">
        <label class="limit-field">
          <span>Startup preset</span>
          <select
            value={lastUsed ? "last" : String(slot)}
            disabled={disabled}
            onChange={(e) => {
              const v = (e.target as HTMLSelectElement).value;
              void session.write("preset.startup", v === "last" ? startupBytes(true, slot) : startupBytes(false, Number(v)));
            }}
          >
            <option value="last">The last one used</option>
            {Array.from({ length: PRESET_SLOTS }, (_, i) => (
              <option key={i} value={String(i)}>
                {slotName(i)}
              </option>
            ))}
          </select>
        </label>
        <label class="limit-field">
          <span>Master volume</span>
          <select value={String(s.masterVolumeMode ?? 0)} disabled={disabled} onChange={(e) => void session.write("vol.master.mode", { choice: Number((e.target as HTMLSelectElement).value) })}>
            {STORAGE_CHOICES.map((c) => (
              <option key={c.value} value={String(c.value)}>
                {c.label}
              </option>
            ))}
          </select>
        </label>
        <label class="limit-field">
          <span>Output configuration</span>
          <select
            value={String((s.outputConfigMode as number | undefined) ?? 1)}
            disabled={disabled}
            onChange={(e) => void session.write("preset.iomode", { choice: Number((e.target as HTMLSelectElement).value) })}
          >
            {STORAGE_CHOICES.map((c) => (
              <option key={c.value} value={String(c.value)}>
                {c.label}
              </option>
            ))}
          </select>
        </label>
      </div>
      {dac?.supported ? (
        <fieldset class="power-dac" disabled={disabled}>
          <legend>DAC hardware mute</legend>
          <label class="limit-check">
            <input type="checkbox" checked={dac.enabled} onChange={(e) => void setDacField({ enabled: (e.target as HTMLInputElement).checked })} /> Mute the DAC
            while the DSPi starts or changes state
          </label>
          {dac.enabled ? (
            <>
              <label class="limit-field">
                <span>Polarity</span>
                <select value={dac.activeLow ? "low" : "high"} onChange={(e) => void setDacField({ activeLow: (e.target as HTMLSelectElement).value === "low" })}>
                  <option value="low">Active low</option>
                  <option value="high">Active high</option>
                </select>
              </label>
              {ms("Hold before the clock stops (ms)", dac.holdMs, (v) => void setDacField({ holdMs: v }))}
              {ms("Wait after unmute (ms)", dac.releaseMs, (v) => void setDacField({ releaseMs: v }))}
              <button type="button" onClick={() => void session.write("dev.dacmute.test", "trigger")}>
                Test Mute
              </button>
            </>
          ) : null}
        </fieldset>
      ) : null}
      <p class="note">S/PDIF and ADAT inputs stay muted until they lock to their signal.</p>
      <div class="installer-buttons">
        <button type="button" disabled={disabled} title="Load the startup preset now, as the DSPi does when it powers on" onClick={() => void test()}>
          Test Power-on
        </button>
        {note ? (
          <span class="note" role="status">
            {note}
          </span>
        ) : null}
      </div>
    </section>
  );
}
