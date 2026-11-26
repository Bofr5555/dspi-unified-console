// WP WB6i: Settings (M55, M61, M62, M56), after the Console's Settings window. Pages:
//
// - Global Parameters: the device's name (M34) and the output latency (M35). The startup preset
//   and what is kept at power-on are the Installer page's (P1WB2), not repeated here.
// - Pins: every GPIO the device reports, each movable by its registry path (the Advanced and
//   Installer modes: the manifest's "hardware" screen). The device refuses a pin it cannot use;
//   this page judges none (the core does not export the pin rules yet).
// - Graphing and Spectrum Analyser: the console's own preferences, applied at once (`model.ts`).
// - Advanced: Reset Channel Names (one undo step), Show Debug Information, the device's facts,
//   and in the Installer mode the way to Core Voltage (the manifest's `core_voltage`).
// - About (-- WP WEB7 -- was Help): who made what, Weeb Labs' links, the credits, the two GitHub links.
//
// Every device change is a core call (`write`, the core-voltage calls), refused while read only.
//
// -- WP WEB7 -- Laid out as Qt Modern's Settings (SettingsPanel, LAY5b, DL7): the parts
// Application / Display / Device / Control as the page's tabs (pages.ts); beside the page a
// list on the sidebar's tone with a search over every part and a tinted badge per page; the
// page's title over its 760 px column; groups as Qt's SettingsGroup (icon + title above the
// panel); rows as form rows with the control at the right edge; sliders with their value in a
// field. Control is the control-surface editor (screens/surfaces), no screen of its own now.

import type { ComponentChildren } from "preact";
import { useEffect, useState } from "preact/hooks";
import type { ModeManifest, OutputLatencyRecord, ParamInfo } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { CoreVoltage } from "./CoreVoltage";
import { SpeakersPage } from "./speakers/SpeakersPage"; // -- WP ROLEW --
import { AppearanceSettings } from "../../ui/arrangement/Appearance"; // -- WP WEBM4 --
import { FormRow, Switch, rangeFill } from "../../ui/design"; // -- WP WEB2 -- -- WP WEB7 -- FormRow, Switch
import { aboutData } from "../../ui/texts"; // -- WP WEB7 -- the About page
import { PageParts, usePart } from "../../ui/shell/Parts"; // -- WP WEB7 -- Qt's parts as the page's tabs
import { placeOf } from "../../ui/shell/places"; // -- WP WEB7 --
import { SurfacesPage, SurfacesSaveBar, csPagesOf, useCsDesk } from "../surfaces/SurfacesScreen"; // -- WP WEB7 --
import type { CsPage } from "../surfaces/model";
import { Group } from "../../ui/Group";
import { Icon } from "../../ui/icons";
import {
  PART_TITLES,
  SETTINGS_PAGE_DEFS,
  isHeading,
  listRows,
  pageDef,
  pageIdOf,
  pageOfPart,
  partsShown,
  type SettingsPageId,
  type SettingsPart,
} from "./pages";
import "./settings.css"; // -- WP WEB7 --
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

// -- WP WEBM4 -- -- WP WEB7 -- by Qt's parts
/** The pages every mode shows, by part: the command palette finds them (Pins, Core Voltage and Control depend on the mode). */
export const SETTINGS_PAGES: readonly { id: string; title: string; group: string }[] = SETTINGS_PAGE_DEFS.filter(
  (p) => !p.hidden && p.id !== "pins" && p.part !== "control",
).map((p) => ({ id: p.id, title: p.title, group: PART_TITLES[p.part] }));

/** -- WP WEB7 -- The palette's pages for `mode`: every mode's, and Control's where the mode has it (not Simple). */
export function settingsPagesFor(mode: string): readonly { id: string; title: string; group: string }[] {
  if (mode === "simple") return SETTINGS_PAGES;
  return [...SETTINGS_PAGES, ...SETTINGS_PAGE_DEFS.filter((p) => p.part === "control").map((p) => ({ id: p.id, title: p.title, group: PART_TITLES[p.part] }))];
}

/** -- WP WEB7 -- The control-surface editor's page for a Control row. */
const CS_PAGE: Partial<Record<SettingsPageId, CsPage>> = { surfaces: "controls", groups: "groups", macros: "macros", aux: "aux" };

export function SettingsScreen(props: ScreenProps) {
  const { session } = props;
  const mode = props.mode ?? "advanced";
  const st = useSessionState(session);
  const [manifest, setManifest] = useState<ModeManifest | null>(null);
  useEffect(() => {
    void session.core.call("modeManifest", undefined).then(setManifest, () => setManifest(null));
  }, [session]);
  // -- WP WEBM4 -- opened at a page (the command palette); -- WP WEB7 -- on that page's part
  const [page, setPageState] = useState<SettingsPageId>(() => {
    const asked = pageIdOf((props.target as { page?: unknown } | null | undefined)?.page);
    if (asked) placeOf(session).parts.set("settings", pageDef(asked).part);
    return asked ?? "global";
  });
  const [query, setQuery] = useState("");
  const [lastOf, setLastOf] = useState<Partial<Record<SettingsPart, SettingsPageId>>>({});
  const pinsShown = screenIn(manifest, "hardware", mode);
  const vregShown = operationIn(manifest, "core_voltage", mode);
  const controlShown = mode !== "simple"; // Control Surfaces: setting up the hardware (not in Simple, as before)
  const [partNow] = usePart(session, "settings", ["application", "display", "device", "control"], "device");
  const desk = useCsDesk(session, controlShown && (partNow === "control" || query.trim() !== ""));
  const csPages = controlShown ? csPagesOf(desk) : [];
  const shown: SettingsPageId[] = [
    "about",
    "advanced",
    "appearance",
    "graphing",
    "spectrum",
    ...(pinsShown ? (["pins"] as const) : []),
    "global",
    "speakers",
    ...(controlShown ? (["surfaces", "groups", "macros", "aux"] as const).filter((p) => csPages.includes(CS_PAGE[p]!)) : []),
    ...(vregShown ? (["vreg"] as const) : []),
  ];
  const parts = partsShown(shown);
  const [partId, setPart] = usePart(session, "settings", parts, "device");
  const part = partId as SettingsPart;
  if (!st.snapshot) return null;
  const current: SettingsPageId = shown.includes(page) && pageDef(page).part === part ? page : (pageOfPart(part, shown, lastOf[part]) ?? "global");
  const open = (id: SettingsPageId) => {
    const g = pageDef(id).part;
    setLastOf((m) => ({ ...m, [g]: id }));
    setPageState(id);
    if (g !== part) setPart(g);
  };
  const choosePart = (id: string) => {
    setPart(id);
    const next = pageOfPart(id as SettingsPart, shown, lastOf[id as SettingsPart]);
    if (next) setPageState(next);
  };
  const rows = listRows(shown, part, query);
  const firstMatch = rows.find((r) => !isHeading(r));
  const def = pageDef(current);
  const cs = CS_PAGE[current];

  const row = (p: (typeof SETTINGS_PAGE_DEFS)[number]) => (
    <button type="button" key={p.id} class="st-row" aria-current={p.id === current ? "page" : undefined} onClick={() => open(p.id)}>
      <span class="st-badge" style={{ background: p.tint }} aria-hidden="true">
        <Icon name={p.icon} />
      </span>
      <span class="st-row-title">{p.title}</span>
    </button>
  );
  const groups: { heading: string | null; pages: (typeof SETTINGS_PAGE_DEFS)[number][] }[] = [];
  for (const r of rows) {
    if (isHeading(r)) groups.push({ heading: r.heading, pages: [] });
    else if (groups.length) groups[groups.length - 1]!.pages.push(r);
    else groups.push({ heading: null, pages: [r] });
  }

  return (
    <div class="settings-screen st-screen">
      <PageParts label="Settings" parts={parts.map((g) => ({ id: g, label: PART_TITLES[g] }))} current={part} onPart={choosePart} />
      <div class="st-side">
        <label class="st-search">
          <Icon name="search" />
          <input
            type="search"
            aria-label="Search settings"
            placeholder="Search"
            value={query}
            onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
            onKeyDown={(e) => {
              // Down and Return open the first page left (Qt's behaviour.md §9); Escape clears
              if ((e.key === "Enter" || e.key === "ArrowDown") && firstMatch && !isHeading(firstMatch)) {
                e.preventDefault();
                open(firstMatch.id);
              } else if (e.key === "Escape" && query) {
                e.preventDefault();
                setQuery("");
              }
            }}
          />
        </label>
        <nav class="settings-nav st-list" aria-label="Settings pages">
          {groups.map((g, i) =>
            g.heading ? (
              <div class="st-list-group" role="group" aria-label={g.heading} key={g.heading}>
                <span class="st-heading" aria-hidden="true">
                  {g.heading}
                </span>
                {g.pages.map(row)}
              </div>
            ) : (
              <div class="st-list-group" key={`plain${i}`}>
                {g.pages.map(row)}
              </div>
            ),
          )}
          {rows.length === 0 ? (
            <p class="st-nomatch" role="status">
              No matching settings
            </p>
          ) : null}
        </nav>
      </div>
      <div class="settings-page st-page" role="tabpanel" aria-label={def.title}>
        {/* -- WP WEB2 -- (DL7) the page's name over it, as Qt's Settings pages */}
        <h2 class="settings-page-title">{def.title}</h2>
        {current === "global" ? <GlobalPage session={session} /> : null}
        {current === "pins" ? <PinsPage session={session} /> : null}
        {current === "speakers" ? <SpeakersPage session={session} /> : null /* -- WP ROLEW -- */}
        {current === "graphing" ? <GraphingPage /> : null}
        {current === "spectrum" ? <SpectrumPage /> : null}
        {current === "advanced" ? <AdvancedPage {...props} vregShown={vregShown} onVreg={() => open("vreg")} /> : null}
        {current === "vreg" ? <CoreVoltage session={session} /> : null}
        {current === "about" ? <AboutPage /> : null}
        {current === "appearance" ? <AppearanceSettings /> : null /* -- WP WEBM4 -- */}
        {cs ? <SurfacesPage desk={desk} page={cs} session={session} /> : null}
        {cs ? <SurfacesSaveBar desk={desk} ask={props.ask} /> : null}
      </div>
    </div>
  );
}

// -- WP UWN -- (UXP A16)
/** The pages Settings shows with no DSPi open: the Application and Display parts, which are this console's own. */
export const NO_DEVICE_PAGES: readonly SettingsPageId[] = ["about", "appearance", "graphing", "spectrum"];

/**
 * Settings with no DSPi open (Qt shows its Application and Display parts then too): the same list and page layout,
 * the console's own pages only; Device and Control wait for a DSPi.
 */
export function NoDeviceSettings() {
  const parts = partsShown(NO_DEVICE_PAGES);
  const [part, setPart] = useState<SettingsPart>("application");
  const [page, setPage] = useState<SettingsPageId>("appearance");
  const current = pageDef(page).part === part ? page : (pageOfPart(part, NO_DEVICE_PAGES, undefined) ?? "about");
  const def = pageDef(current);
  const rows = listRows(NO_DEVICE_PAGES, part, "").filter((r) => !isHeading(r)) as (typeof SETTINGS_PAGE_DEFS)[number][];
  return (
    <div class="settings-screen st-screen">
      <PageParts label="Settings" parts={parts.map((g) => ({ id: g, label: PART_TITLES[g] }))} current={part} onPart={(id) => setPart(id as SettingsPart)} />
      <div class="st-side">
        <nav class="settings-nav st-list" aria-label="Settings pages">
          <div class="st-list-group">
            {rows.map((p) => (
              <button type="button" key={p.id} class="st-row" aria-current={p.id === current ? "page" : undefined} onClick={() => setPage(p.id)}>
                <span class="st-badge" style={{ background: p.tint }} aria-hidden="true">
                  <Icon name={p.icon} />
                </span>
                <span class="st-row-title">{p.title}</span>
              </button>
            ))}
          </div>
          <p class="note st-nodevice">Open a DSPi for its Device settings.</p>
        </nav>
      </div>
      <div class="settings-page st-page" role="tabpanel" aria-label={def.title}>
        <h2 class="settings-page-title">{def.title}</h2>
        {current === "about" ? <AboutPage /> : null}
        {current === "appearance" ? <AppearanceSettings /> : null}
        {current === "graphing" ? <GraphingPage /> : null}
        {current === "spectrum" ? <SpectrumPage /> : null}
      </div>
    </div>
  );
}

/** -- WP WEB7 -- A row with a switch at the right edge (Qt's SettingsToggle): the title, a caption under it. */
function SwitchRow(props: { label: string; caption?: string; on: boolean; disabled?: boolean; onChange: (on: boolean) => void }) {
  return (
    <FormRow title={props.label} caption={props.caption} class={props.disabled ? "st-off" : undefined}>
      <Switch label={props.label} on={props.on} disabled={props.disabled} toggle={() => props.onChange(!props.on)} />
    </FormRow>
  );
}

/** -- WP WEB7 -- A slider row (Qt's SettingsSliderRow in Modern): the title, the slider, the value in its field at the right. */
function SliderRow(props: { label: string; caption?: string; value: number; text: string; min: number; max: number; step: number; disabled?: boolean; onChange: (v: number) => void }) {
  return (
    <div class={`form-row st-slider${props.disabled ? " st-off" : ""}`}>
      <div class="form-row-text">
        <span class="form-row-title">{props.label}</span>
        {props.caption ? <span class="form-row-caption">{props.caption}</span> : null}
      </div>
      <input
        type="range"
        aria-label={props.label}
        min={props.min}
        max={props.max}
        step={props.step}
        value={props.value}
        style={rangeFill(props.value, props.min, props.max)}
        disabled={props.disabled}
        onInput={(e) => props.onChange(Number((e.target as HTMLInputElement).value))}
      />
      <output class="st-value" aria-label={`${props.label} value`}>
        {props.text}
      </output>
    </div>
  );
}

/** -- WP WEB7 -- A picker row (Qt's SettingsPickerRow): the title, the list at the right edge. */
function PickerRow(props: { label: string; caption?: string; values: number[]; text: (v: number) => string; value: number; disabled?: boolean; onChange: (v: number) => void }) {
  return (
    <FormRow title={props.label} caption={props.caption}>
      <select aria-label={props.label} value={props.value} disabled={props.disabled} onChange={(e) => props.onChange(Number((e.target as HTMLSelectElement).value))}>
        {props.values.map((v) => (
          <option key={v} value={v}>
            {props.text(v)}
          </option>
        ))}
      </select>
    </FormRow>
  );
}

/** -- WP WEB7 -- A value row (Qt's SettingsValueRow): the title, the value at the right edge. */
function ValueRow(props: { label: string; children: ComponentChildren; mono?: boolean }) {
  return (
    <FormRow title={props.label} class="st-value-row">
      <span class={props.mono ? "st-fact mono" : "st-fact"}>{props.children}</span>
    </FormRow>
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
        <Group
          title="Device Name"
          icon={<Icon name="cpu" />}
          footer="Stored on the device, so it keeps this name with any console or DSPi Link hub. Shown in the device picker and on the front panel."
        >
          <FormRow title="Name" caption={st.snapshot?.device.serial ? `Serial ${st.snapshot.device.serial}` : undefined}>
            <input
              type="text"
              class="st-field"
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
            <button type="button" disabled={ro || draft === null} onClick={() => void rename()}>
              Rename
            </button>
            {name ? (
              <button type="button" disabled={ro} onClick={() => void session.write("dev.name", { text: "" }).then((ok) => ok && setDraft(null))}>
                Remove the name
              </button>
            ) : null}
          </FormRow>
          {nameProblem ? (
            <p class="error" role="alert">
              {nameProblem}
            </p>
          ) : null}
        </Group>
      ) : null}
      {latency?.supported ? (
        <Group title="Latency" icon="timer" footer={<span class="settings-latency-caption">{latencyCaption(latency)}</span>}>
          <FormRow title="Target">
            <select aria-label="Latency target" value={latency.requestedId} disabled={ro} onChange={(e) => void choose(Number((e.target as HTMLSelectElement).value))}>
              {latencyChoices(latency).map((o) => (
                <option key={o.id} value={o.id}>
                  {o.text}
                </option>
              ))}
            </select>
          </FormRow>
        </Group>
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
    <Group
      title="Pin Assignments"
      icon={<Icon name="cpu" />}
      footer="Changing a pin restarts the audio that uses it. The device refuses a pin it cannot use for the job or that something else holds, and says so above."
    >
      <table class="settings-pins ds-table">
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
    </Group>
  );
}

// ---------------------------------------------------------------- Graphing and Spectrum

function GraphingPage() {
  const store = prefsStore();
  const g = usePrefs(store).graph;
  const set = (patch: Partial<GraphPrefs>) => store.set({ graph: patch });
  const [lo, hi] = dbSpan(g);
  return (
    <>
      <Group title="Graph Appearance" icon="chart-line">
        <SwitchRow label="Graph Line Glow" caption="Add a neon glow effect to frequency response curves" on={g.showGlow} onChange={(v) => set({ showGlow: v })} />
        {/* -- WP GAP1 -- Qt's phase switches (the Equaliser graph's Phase chip is the first) */}
        <SwitchRow label="Show Phase Response" caption="Overlay the selected channel's phase (degrees) as a dotted line" on={g.showPhase} onChange={(v) => set({ showPhase: v })} />
        <SwitchRow
          label="Unwrap Phase"
          caption="Show continuous phase instead of wrapping at ±180°"
          on={g.phaseUnwrapped}
          disabled={!g.showPhase}
          onChange={(v) => set({ phaseUnwrapped: v })}
        />
      </Group>
      <Group title="Response Curve" icon={<Icon name="activity" />}>
        <SliderRow label="Line Width" value={g.lineWidth} text={`${g.lineWidth.toFixed(1)} pt`} min={1} max={4} step={0.5} onChange={(v) => set({ lineWidth: v })} />
      </Group>
      <Group title="Scale & Grid" icon={<Icon name="layout-grid" />}>
        <SwitchRow label="Show Frequency Grid" on={g.showFreqGrid} onChange={(v) => set({ showFreqGrid: v })} />
        <SwitchRow label="Show Frequency Labels" on={g.showFreqLabels} onChange={(v) => set({ showFreqLabels: v })} />
        <SwitchRow label="Show dB Grid" on={g.showDbGrid} onChange={(v) => set({ showDbGrid: v })} />
        <SwitchRow label="Show dB Labels" on={g.showDbLabels} onChange={(v) => set({ showDbLabels: v })} />
        <SwitchRow label="Show Frequency Readout" on={g.showFreqReadout} onChange={(v) => set({ showFreqReadout: v })} />
        <SwitchRow label="Show Gain Readout" on={g.showLevelReadout} onChange={(v) => set({ showLevelReadout: v })} />
        <SliderRow
          label="Grid Opacity"
          value={g.gridOpacity}
          text={`${Math.round(g.gridOpacity * 100)} %`}
          min={0}
          max={2}
          step={0.05}
          disabled={!g.showFreqGrid && !g.showDbGrid}
          onChange={(v) => set({ gridOpacity: v })}
        />
        <SliderRow label="Vertical Range" value={g.dbRange} text={`${g.dbRange} dB`} min={10} max={100} step={1} onChange={(v) => set({ dbRange: Math.round(v) })} />
        <SliderRow
          label="Center"
          caption={`Shows ${signedDb(hi)} to ${signedDb(lo)} dB`}
          value={g.dbCenter}
          text={`${g.dbCenter} dB`}
          min={-40}
          max={20}
          step={1}
          onChange={(v) => set({ dbCenter: Math.round(v) })}
        />
        <PickerRow label="Min Frequency" values={MIN_FREQS} text={freqText} value={g.minFreq} onChange={(v) => set({ minFreq: v })} />
        <PickerRow label="Max Frequency" values={MAX_FREQS} text={freqText} value={g.maxFreq} onChange={(v) => set({ maxFreq: v })} />
      </Group>
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
      <Group title="Display" icon={<Icon name="layout-grid" />}>
        <SliderRow
          label="Spectrum Strength"
          caption="How far the fill comes forward behind the curves when the spectrum is drawn on the response graph."
          value={s.graphOpacity}
          text={`${Math.round(s.graphOpacity * 100)} %`}
          min={0.3}
          max={1}
          step={0.05}
          onChange={(v) => set({ graphOpacity: v })}
        />
        <SwitchRow label="Peak Hold" caption="A cap above each band marking its recent maximum" on={s.showPeakHold} onChange={(v) => set({ showPeakHold: v })} />
        <SwitchRow label="Smoothing" caption="Glides the spectrum between device frames" on={s.smoothing} onChange={(v) => set({ smoothing: v })} />
      </Group>
      <Group title="Vertical Scale" icon={<Icon name="sliders-vertical" />} footer="Six decibels of headroom above full scale is the default because upmix-derived rows and hot EQ can legitimately exceed 0 dBFS.">
        <PickerRow label="Floor" values={FLOORS} text={(v) => `${v} dB`} value={s.floorDb} onChange={(v) => set({ floorDb: v })} />
        <PickerRow label="Ceiling" values={CEILINGS} text={(v) => (v ? `+${v} dBFS` : "0 dBFS")} value={s.ceilingDb} onChange={(v) => set({ ceilingDb: v })} />
      </Group>
      <Group title="Engine" icon="cpu">
        <PickerRow
          label="Transform Size"
          caption="More points resolve lower frequencies, but a frame takes longer to fill, so each channel refreshes less often."
          values={FFT_ORDERS}
          text={(o) => `${1 << o} points`}
          value={s.fftOrder}
          onChange={(v) => set({ fftOrder: v })}
        />
        <PickerRow label="Averaging" values={AVERAGING} text={(v) => (v === 0 ? "Off" : v >= 1000 ? `${v / 1000} s` : `${v} ms`)} value={s.avgMs} onChange={(v) => set({ avgMs: v })} />
        <PickerRow label="Peak Decay" values={PEAK_DECAYS} text={(v) => (v === 0 ? "Off" : `${v} dB/s`)} value={s.peakDecay} disabled={!s.showPeakHold} onChange={(v) => set({ peakDecay: v })} />
      </Group>
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
      <Group title="Channel Names" icon={<Icon name="list-ordered" />}>
        <FormRow title="Reset all channel names to factory defaults.">
          <button type="button" disabled={ro || !defaultChannelNames(s, snap.outputLabels)} onClick={() => void resetNames()}>
            Reset
          </button>
        </FormRow>
      </Group>
      <Group title="Diagnostics" icon={<Icon name="activity" />}>
        <SwitchRow label="Show Debug Information" caption="Display additional diagnostic data in the UI" on={prefs.debug} onChange={(v) => store.set({ debug: v })} />
      </Group>
      <Group title="Device" icon={<Icon name="hard-drive" />}>
        <ValueRow label="Platform">{snap.device.platform || "-"}</ValueRow>
        <ValueRow label="Firmware">{snap.device.firmware || "-"}</ValueRow>
        <ValueRow label="Serial Number" mono>
          {snap.device.serial || "-"}
        </ValueRow>
        <ValueRow label="Channels">
          {s.numInputChannels} in, {s.numOutputChannels} out
        </ValueRow>
      </Group>
      {prefs.debug ? (
        <Group title="Debug Information" icon={<Icon name="activity" />}>
          <dl class="settings-facts mono st-facts" aria-label="Debug facts">
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
        </Group>
      ) : null}
      {props.vregShown ? (
        <Group title="Core Voltage" icon={<Icon name="gauge" />}>
          <FormRow title={vreg ? (vreg.available ? `Running at ${voltsText(vreg.mv)}${vreg.trial ? " (trial)" : ""}` : "Not offered by this DSPi.") : "Reading…"}>
            <button type="button" onClick={props.onVreg}>
              Open…
            </button>
          </FormRow>
        </Group>
      ) : (
        <p class="note">Core Voltage is in the Installer mode.</p>
      )}
    </>
  );
}

// ---------------------------------------------------------------- About (-- WP WEB7 -- was Help)

/** Qt's About page as far as the web has its data (assets/about.json, the help links). */
function AboutPage() {
  const a = aboutData;
  const ext = (url: string, title: ComponentChildren, detail?: string) => (
    <p class="st-link">
      <a href={url} target="_blank" rel="noopener noreferrer">
        <Icon name="external-link" />
        {title}
      </a>
      {detail ? <span class="st-link-detail">{detail}</span> : null}
    </p>
  );
  return (
    <>
      <Group class="st-about-head">
        <div class="st-app">
          <span class="st-app-icon" aria-hidden="true">
            <Icon name="volume-2" />
          </span>
          <div class="st-app-text">
            <strong>uConsole for DSPi</strong>
            <span>USB Audio DSP Controller</span>
            {a.credit_lines.map((l) => (
              <span key={l.id}>{l.text}</span>
            ))}
          </div>
        </div>
      </Group>
      <Group title="Made By" icon={<Icon name="heart" />}>
        {a.makers.map((m) => (
          <div class="st-maker" key={m.id}>
            {ext(m.url, m.name)}
            <span class="st-link-detail">{m.made.join(", ")}</span>
          </div>
        ))}
        {HELP_LINKS.map((l) => (
          <div key={l.id}>{ext(l.url, l.title)}</div>
        ))}
      </Group>
      <Group title={a.links_title || "Links & Support"} icon={<Icon name="external-link" />}>
        <p class="st-note">{a.links_note}</p>
        {a.links.map((l) => (
          <div key={l.id}>{ext(l.url, l.title, l.detail)}</div>
        ))}
      </Group>
      <Group title="Credits" icon={<Icon name="star" />}>
        {a.credits.map((c) => (
          <div class="st-credit" key={c.id}>
            {c.url ? (
              <a href={c.url} target="_blank" rel="noopener noreferrer">
                {c.name}
              </a>
            ) : (
              <strong>{c.name}</strong>
            )}
            <span class="st-link-detail">{c.what}</span>
          </div>
        ))}
      </Group>
      <Group title="Third-Party Notices" icon={<Icon name="file-down" />}>
        {ext("THIRD-PARTY-NOTICES.txt", "Third-party notices", "The licences of the libraries this console includes")}
      </Group>
    </>
  );
}
