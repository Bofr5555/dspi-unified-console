// WP WB6i: Settings (M55, M61, M62, M56), after the Console's Settings window. Pages:
//
// - Global: the device's name (M34) and the output latency (M35). The startup preset and what
//   is kept at power-on are the Installer page's (P1WB2), not repeated here.
// - Pins: every GPIO the device reports, each movable by its registry path (the Advanced and
//   Installer modes: the manifest's "hardware" screen). The device refuses a pin it cannot use;
//   this page judges none (the core does not export the pin rules yet).
// - Graphing and Spectrum: the console's own preferences, applied at once (`model.ts`).
// - Advanced: Reset Channel Names (one undo step), Show Debug Information, the device's facts,
//   and in the Installer mode the way to Core Voltage (the manifest's `core_voltage`).
// - Help: the two GitHub links.
//
// Every device change is a core call (`write`, the core-voltage calls), refused while read only.

import type { ComponentChildren } from "preact";
import { useEffect, useState } from "preact/hooks";
import type { ModeManifest, OutputLatencyRecord, ParamInfo } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { CoreVoltage } from "./CoreVoltage";
import {
  AVERAGING,
  CEILINGS,
  FFT_ORDERS,
  FLOORS,
  HELP_LINKS,
  MAX_FREQS,
  MIN_FREQS,
  PEAK_DECAYS,
  dbSpan,
  defaultChannelNames,
  deviceNameValue,
  freqText,
  latencyCaption,
  latencyChoices,
  resetChannelNames,
  operationIn,
  pinRows,
  pinText,
  pinValue,
  prefsStore,
  screenIn,
  signedDb,
  usePrefs,
  voltsText,
  type GraphPrefs,
  type SpectrumPrefs,
} from "./model";

type Page = "global" | "pins" | "graphing" | "spectrum" | "advanced" | "help" | "vreg";

const PAGE_TITLES: Record<Page, string> = {
  global: "Global",
  pins: "Pins",
  graphing: "Graphing",
  spectrum: "Spectrum",
  advanced: "Advanced",
  help: "Help",
  vreg: "Core Voltage",
};

export function SettingsScreen(props: ScreenProps) {
  const { session } = props;
  const mode = props.mode ?? "advanced";
  const st = useSessionState(session);
  const [manifest, setManifest] = useState<ModeManifest | null>(null);
  useEffect(() => {
    void session.core.call("modeManifest", undefined).then(setManifest, () => setManifest(null));
  }, [session]);
  const [page, setPage] = useState<Page>("global");
  if (!st.snapshot) return null;
  const pinsShown = screenIn(manifest, "hardware", mode);
  const vregShown = operationIn(manifest, "core_voltage", mode);
  const pages: Page[] = ["global", ...(pinsShown ? (["pins"] as Page[]) : []), "graphing", "spectrum", "advanced", ...(vregShown ? (["vreg"] as Page[]) : []), "help"];
  const current = pages.includes(page) ? page : "global";
  return (
    <div class="settings-screen">
      <nav class="settings-nav" aria-label="Settings pages">
        {pages.map((p) => (
          <button type="button" key={p} aria-current={p === current ? "page" : undefined} onClick={() => setPage(p)}>
            {PAGE_TITLES[p]}
          </button>
        ))}
      </nav>
      {current === "global" ? <GlobalPage session={session} /> : null}
      {current === "pins" ? <PinsPage session={session} /> : null}
      {current === "graphing" ? <GraphingPage /> : null}
      {current === "spectrum" ? <SpectrumPage /> : null}
      {current === "advanced" ? <AdvancedPage {...props} vregShown={vregShown} onVreg={() => setPage("vreg")} /> : null}
      {current === "vreg" ? <CoreVoltage session={session} /> : null}
      {current === "help" ? <HelpPage /> : null}
    </div>
  );
}

function Card(props: { label: string; children: ComponentChildren; note?: string }) {
  return (
    <section class="card settings-card" aria-label={props.label}>
      <h3>{props.label}</h3>
      {props.children}
      {props.note ? <p class="note">{props.note}</p> : null}
    </section>
  );
}

// ---------------------------------------------------------------- Global

function GlobalPage(props: { session: Session }) {
  const { session } = props;
  const st = useSessionState(session);
  const ro = !!session.readOnlyReason;
  const [named, setNamed] = useState<boolean | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const [latency, setLatency] = useState<OutputLatencyRecord | null>(null);
  const [nameProblem, setNameProblem] = useState("");
  const name = st.snapshot?.device.name ?? "";

  const readLatency = () =>
    session.core.call("outputLatency", undefined).then(
      (r) => {
        setLatency(r);
        return r;
      },
      () => {
        setLatency(null);
        return null;
      },
    );
  useEffect(() => {
    void session.core.call("deviceName", undefined).then(
      (r) => setNamed(r.supported),
      () => setNamed(false),
    );
    void readLatency();
  }, [session]);

  const rename = async () => {
    if (draft === null) return;
    const v = deviceNameValue(draft);
    if (v === null) {
      setNameProblem("A name holds at most 31 bytes.");
      return;
    }
    setNameProblem("");
    if (v !== name && (await session.write("dev.name", { text: v }))) setDraft(null);
    else if (v === name) setDraft(null);
  };

  const choose = async (id: number) => {
    if (!(await session.write("out.latency", { choice: id }))) return;
    // The device applies it from its main loop: read until it is no longer pending.
    for (let i = 0; i < 20; i++) {
      const r = await readLatency();
      if (!r?.pending) return;
      await new Promise((res) => setTimeout(res, 500));
    }
  };

  return (
    <>
      {named ? (
        <Card label="Device Name" note="Stored on the device, so it keeps this name with any console or DSPi Link hub. Shown in the device picker and on the front panel.">
          <div class="settings-row">
            <label>
              Name{" "}
              <input
                type="text"
                aria-label="Device Name"
                maxLength={31}
                value={draft ?? name}
                disabled={ro}
                placeholder="No name"
                onInput={(e) => setDraft((e.target as HTMLInputElement).value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void rename();
                  if (e.key === "Escape") setDraft(null);
                }}
              />
            </label>
            <button type="button" disabled={ro || draft === null} onClick={() => void rename()}>
              Rename
            </button>
            {name ? (
              <button type="button" disabled={ro} onClick={() => void session.write("dev.name", { text: "" }).then((ok) => ok && setDraft(null))}>
                Remove the name
              </button>
            ) : null}
          </div>
          {st.snapshot?.device.serial ? <p class="note">Serial {st.snapshot.device.serial}</p> : null}
          {nameProblem ? (
            <p class="error" role="alert">
              {nameProblem}
            </p>
          ) : null}
        </Card>
      ) : null}
      {latency?.supported ? (
        <Card label="Latency">
          <div class="settings-row">
            <label>
              Target{" "}
              <select aria-label="Latency target" value={latency.requestedId} disabled={ro} onChange={(e) => void choose(Number((e.target as HTMLSelectElement).value))}>
                {latencyChoices(latency).map((o) => (
                  <option key={o.id} value={o.id}>
                    {o.text}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <p class="note settings-latency-caption">{latencyCaption(latency)}</p>
        </Card>
      ) : null}
      {named === false && latency && !latency.supported ? <p class="note">This firmware has no device name and no latency setting.</p> : null}
      <p class="note">The startup preset and what the device keeps at power-on are on the Installer page.</p>
    </>
  );
}

// ---------------------------------------------------------------- Pins

function PinsPage(props: { session: Session }) {
  const { session } = props;
  const st = useSessionState(session);
  const ro = !!session.readOnlyReason;
  const rows = st.snapshot ? pinRows(st.snapshot.state) : [];
  const paths = [...new Set(rows.map((r) => r.path))].join(",");
  const [info, setInfo] = useState<Record<string, ParamInfo>>({});
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  useEffect(() => {
    let alive = true;
    for (const p of paths.split(",").filter(Boolean)) {
      void session.core.call("describe", { path: p }).then(
        (d) => alive && setInfo((m) => ({ ...m, [p]: d })),
        () => undefined,
      );
    }
    return () => {
      alive = false;
    };
  }, [session, paths]);

  const move = async (key: string, path: string, indices: number[], text: string) => {
    const v = pinValue(text);
    if (!v) return;
    if (await session.write(path, v, indices.length ? indices : undefined)) setDrafts((d) => ({ ...d, [key]: "" }));
  };

  return (
    <Card
      label="Pins"
      note="Changing a pin restarts the audio that uses it. The device refuses a pin it cannot use for the job or that something else holds, and says so above."
    >
      <table class="settings-pins">
        <thead>
          <tr>
            <th scope="col">Use</th>
            <th scope="col">Pin</th>
            <th scope="col">Move to</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => {
            const d = info[r.path];
            const draft = drafts[r.key] ?? "";
            return (
              <tr key={r.key}>
                <th scope="row" title={d?.plain}>
                  {r.title}
                </th>
                <td>{pinText(r.pin)}</td>
                <td>
                  <input
                    type="number"
                    inputMode="numeric"
                    aria-label={`${r.title}: new GPIO`}
                    min={d?.min}
                    max={d?.max}
                    step={1}
                    value={draft}
                    disabled={ro}
                    placeholder={r.pin === 255 ? "" : String(r.pin)}
                    onInput={(e) => setDrafts((m) => ({ ...m, [r.key]: (e.target as HTMLInputElement).value }))}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") void move(r.key, r.path, r.indices, draft);
                    }}
                  />{" "}
                  <button type="button" disabled={ro || !pinValue(draft)} onClick={() => void move(r.key, r.path, r.indices, draft)}>
                    Move
                  </button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </Card>
  );
}

// ---------------------------------------------------------------- Graphing and Spectrum

function Toggle(props: { label: string; caption?: string; checked: boolean; disabled?: boolean; onChange: (on: boolean) => void }) {
  return (
    <label class="settings-toggle">
      <input type="checkbox" checked={props.checked} disabled={props.disabled} onChange={(e) => props.onChange((e.target as HTMLInputElement).checked)} />
      <span>
        {props.label}
        {props.caption ? <small>{props.caption}</small> : null}
      </span>
    </label>
  );
}

function Slider(props: { label: string; min: number; max: number; step: number; value: number; disabled?: boolean; onChange: (v: number) => void }) {
  return (
    <label class="settings-slider">
      <span>{props.label}</span>
      <input
        type="range"
        aria-label={props.label.replace(/:.*$/, "")}
        min={props.min}
        max={props.max}
        step={props.step}
        value={props.value}
        disabled={props.disabled}
        onInput={(e) => props.onChange(Number((e.target as HTMLInputElement).value))}
      />
    </label>
  );
}

function Picker(props: { label: string; values: number[]; text: (v: number) => string; value: number; disabled?: boolean; onChange: (v: number) => void }) {
  return (
    <label class="settings-picker">
      {props.label}{" "}
      <select value={props.value} disabled={props.disabled} onChange={(e) => props.onChange(Number((e.target as HTMLSelectElement).value))}>
        {props.values.map((v) => (
          <option key={v} value={v}>
            {props.text(v)}
          </option>
        ))}
      </select>
    </label>
  );
}

function GraphingPage() {
  const store = prefsStore();
  const g = usePrefs(store).graph;
  const set = (patch: Partial<GraphPrefs>) => store.set({ graph: patch });
  const [lo, hi] = dbSpan(g);
  return (
    <>
      <Card label="Graph Appearance">
        <Toggle label="Graph Line Glow" caption="Add a neon glow effect to frequency response curves" checked={g.showGlow} onChange={(v) => set({ showGlow: v })} />
      </Card>
      <Card label="Response Curve">
        <Slider label={`Line Width: ${g.lineWidth.toFixed(1)}pt`} min={1} max={4} step={0.5} value={g.lineWidth} onChange={(v) => set({ lineWidth: v })} />
      </Card>
      <Card label="Scale & Grid">
        <Toggle label="Show Frequency Grid" checked={g.showFreqGrid} onChange={(v) => set({ showFreqGrid: v })} />
        <Toggle label="Show Frequency Labels" checked={g.showFreqLabels} onChange={(v) => set({ showFreqLabels: v })} />
        <Toggle label="Show dB Grid" checked={g.showDbGrid} onChange={(v) => set({ showDbGrid: v })} />
        <Toggle label="Show dB Labels" checked={g.showDbLabels} onChange={(v) => set({ showDbLabels: v })} />
        <Toggle label="Show Frequency Readout" checked={g.showFreqReadout} onChange={(v) => set({ showFreqReadout: v })} />
        <Toggle label="Show Gain Readout" checked={g.showLevelReadout} onChange={(v) => set({ showLevelReadout: v })} />
        <Slider
          label={`Grid Opacity: ${Math.round(g.gridOpacity * 100)}%`}
          min={0}
          max={2}
          step={0.05}
          value={g.gridOpacity}
          disabled={!g.showFreqGrid && !g.showDbGrid}
          onChange={(v) => set({ gridOpacity: v })}
        />
        <Slider label={`Vertical Range: ${g.dbRange} dB`} min={10} max={100} step={1} value={g.dbRange} onChange={(v) => set({ dbRange: Math.round(v) })} />
        <Slider
          label={`Center: ${g.dbCenter} dB → ${signedDb(hi)} to ${signedDb(lo)}`}
          min={-40}
          max={20}
          step={1}
          value={g.dbCenter}
          onChange={(v) => set({ dbCenter: Math.round(v) })}
        />
        <div class="settings-row">
          <Picker label="Min Frequency" values={MIN_FREQS} text={freqText} value={g.minFreq} onChange={(v) => set({ minFreq: v })} />
          <Picker label="Max Frequency" values={MAX_FREQS} text={freqText} value={g.maxFreq} onChange={(v) => set({ maxFreq: v })} />
        </div>
      </Card>
      <div class="settings-buttons">
        <button type="button" onClick={() => store.reset("graph")}>
          Restore Defaults
        </button>
      </div>
    </>
  );
}

function SpectrumPage() {
  const store = prefsStore();
  const s = usePrefs(store).spectrum;
  const set = (patch: Partial<SpectrumPrefs>) => store.set({ spectrum: patch });
  return (
    <>
      <p class="note">This console keeps these for its spectrum analyser, which it does not draw yet.</p>
      <Card label="Display">
        <Slider
          label={`Spectrum Strength: ${Math.round(s.graphOpacity * 100)}%`}
          min={0.3}
          max={1}
          step={0.05}
          value={s.graphOpacity}
          onChange={(v) => set({ graphOpacity: v })}
        />
        <Toggle label="Peak Hold" caption="A cap above each band marking its recent maximum" checked={s.showPeakHold} onChange={(v) => set({ showPeakHold: v })} />
        <Toggle label="Smoothing" caption="Glides the spectrum between device frames" checked={s.smoothing} onChange={(v) => set({ smoothing: v })} />
      </Card>
      <Card label="Vertical Scale" note="Six decibels of headroom above full scale is the default because upmix-derived rows and hot EQ can legitimately exceed 0 dBFS.">
        <div class="settings-row">
          <Picker label="Floor" values={FLOORS} text={(v) => `${v} dB`} value={s.floorDb} onChange={(v) => set({ floorDb: v })} />
          <Picker label="Ceiling" values={CEILINGS} text={(v) => (v ? `+${v} dBFS` : "0 dBFS")} value={s.ceilingDb} onChange={(v) => set({ ceilingDb: v })} />
        </div>
      </Card>
      <Card label="Engine" note="More points resolve lower frequencies, but a frame takes longer to fill, so each channel refreshes less often.">
        <div class="settings-row">
          <Picker label="Transform Size" values={FFT_ORDERS} text={(o) => `${1 << o} points`} value={s.fftOrder} onChange={(v) => set({ fftOrder: v })} />
          <Picker label="Averaging" values={AVERAGING} text={(v) => (v === 0 ? "Off" : v >= 1000 ? `${v / 1000} s` : `${v} ms`)} value={s.avgMs} onChange={(v) => set({ avgMs: v })} />
          <Picker
            label="Peak Decay"
            values={PEAK_DECAYS}
            text={(v) => (v === 0 ? "Off" : `${v} dB/s`)}
            value={s.peakDecay}
            disabled={!s.showPeakHold}
            onChange={(v) => set({ peakDecay: v })}
          />
        </div>
      </Card>
      <div class="settings-buttons">
        <button type="button" onClick={() => store.reset("spectrum")}>
          Restore Defaults
        </button>
      </div>
    </>
  );
}

// ---------------------------------------------------------------- Advanced

function AdvancedPage(props: ScreenProps & { vregShown: boolean; onVreg: () => void }) {
  const { session, ask } = props;
  const st = useSessionState(session);
  const store = prefsStore();
  const prefs = usePrefs(store);
  const snap = st.snapshot;
  const ro = !!session.readOnlyReason;
  const [vreg, setVreg] = useState<{ available: boolean; mv: number; trial: boolean } | null>(null);
  useEffect(() => {
    if (!props.vregShown) return;
    void session.core.call("vregPoll", undefined).then(
      (v) => setVreg({ available: v.available, mv: v.status?.millivolts ?? 0, trial: !!v.status?.trialActive }),
      () => setVreg(null),
    );
  }, [session, props.vregShown]);
  if (!snap) return null;
  const s = snap.state;

  const resetNames = async () => {
    const names = defaultChannelNames(s, snap.outputLabels);
    if (!names) return;
    const a = await ask({
      title: "Reset Channel Names",
      body: <p>Reset all channel names to factory defaults?</p>,
      buttons: [
        { label: "Reset", value: "reset", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a === "reset") await resetChannelNames(session);
  };

  return (
    <>
      <Card label="Channel Names">
        <div class="settings-row">
          <span>Reset all channel names to factory defaults.</span>
          <button type="button" disabled={ro || !defaultChannelNames(s, snap.outputLabels)} onClick={() => void resetNames()}>
            Reset
          </button>
        </div>
      </Card>
      <Card label="Diagnostics">
        <Toggle label="Show Debug Information" caption="Display additional diagnostic data in the UI" checked={prefs.debug} onChange={(v) => store.set({ debug: v })} />
      </Card>
      <Card label="Device">
        <dl class="settings-facts">
          <dt>Platform</dt>
          <dd>{snap.device.platform || "-"}</dd>
          <dt>Firmware</dt>
          <dd>{snap.device.firmware || "-"}</dd>
          <dt>Serial Number</dt>
          <dd class="mono">{snap.device.serial || "-"}</dd>
          <dt>Channels</dt>
          <dd>
            {s.numInputChannels} in, {s.numOutputChannels} out
          </dd>
        </dl>
      </Card>
      {prefs.debug ? (
        <Card label="Debug Information">
          <dl class="settings-facts mono" aria-label="Debug facts">
            <dt>Session</dt>
            <dd>{session.id}</dd>
            <dt>Source</dt>
            <dd>{session.source.kind}</dd>
            <dt>Mode</dt>
            <dd>{props.mode ?? "-"}</dd>
            <dt>DSPi Link</dt>
            <dd>{st.link?.phase ?? "-"}</dd>
            <dt>Read only</dt>
            <dd>{session.readOnlyReason || "no"}</dd>
            <dt>Wire format</dt>
            <dd>{s.formatVersion}</dd>
            <dt>Firmware build</dt>
            <dd>
              {s.fwMajor}.{s.fwMinor}.{s.fwPatch}
              {s.fwBeta ? ` beta ${s.fwBeta}` : ""}
            </dd>
            <dt>Platform id</dt>
            <dd>{s.platformId}</dd>
            <dt>Channels</dt>
            <dd>
              {s.numChannels} ({s.numInputChannels} in, {s.numOutputChannels} out), {s.maxBands} bands
            </dd>
            <dt>Preset</dt>
            <dd>
              slot {s.activePresetSlot + 1}
              {st.unsaved.length ? `, ${st.unsaved.length} unsaved` : ""}
            </dd>
            <dt>Last error</dt>
            <dd>{st.errorNotice?.key ?? (st.error || "none")}</dd>
          </dl>
        </Card>
      ) : null}
      {props.vregShown ? (
        <Card label="Core Voltage">
          <div class="settings-row">
            <span>{vreg ? (vreg.available ? `Running at ${voltsText(vreg.mv)}${vreg.trial ? " (trial)" : ""}` : "Not offered by this DSPi.") : "Reading…"}</span>
            <button type="button" onClick={props.onVreg}>
              Open…
            </button>
          </div>
        </Card>
      ) : (
        <p class="note">Core Voltage is in the Installer mode.</p>
      )}
    </>
  );
}

// ---------------------------------------------------------------- Help

function HelpPage() {
  return (
    <Card label="Help">
      <ul class="settings-links">
        {HELP_LINKS.map((l) => (
          <li key={l.id}>
            <a href={l.url} target="_blank" rel="noopener noreferrer">
              {l.title}
            </a>
          </li>
        ))}
      </ul>
    </Card>
  );
}
