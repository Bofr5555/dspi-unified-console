// WP WB6l: the control-surface editor (CG20), after the Console's
// ControlSurfacesSettingsTab and Qt's Settings > Control Surfaces, Groups,
// Macros and Auxiliary Outputs (WP QP12). One card per wired component, group
// or macro, collapsed to its summary; a card's rows are the ones the core
// describes for its draft (`csDescribe`), every change is the core's next draft
// (`csEdit`) and nothing reaches the device until Apply (`csApply`). The device
// runs an applied change at once as a live preview; Save writes it to flash and
// Revert reloads what is stored. Read only: everything shows, nothing can be
// changed, and the core refuses a write that got past the page.

import type { ComponentChildren } from "preact";
import { useEffect, useMemo, useReducer, useState } from "preact/hooks";
import type { CsRecord, CsRow } from "../../core/protocol";
import type { ScreenProps } from "../../ui/screens";
import { CsDesk } from "./desk";
import { rangeFill } from "../../ui/design"; // -- WP WEB2 --
import {
  PAGE_LABELS,
  TY,
  activePages,
  addableTypes,
  auxOf,
  codeText,
  deviceDirty,
  freePage,
  freeSlot,
  isAux,
  isDirty,
  keysFor,
  liveConfigured,
  macroRunning,
  macroSummary,
  maxSteps,
  nameOf,
  num,
  pagesOf,
  panelState,
  pillOf,
  rowsOf,
  slotActive,
  slotCount,
  summaryOf,
  supported,
  typeName,
  type CsPage,
  type CsRec,
} from "./model";

/** The desk of this screen, read again when the screen shows; a learn waiting for a press ends when it goes. */
function useDesk(props: ScreenProps): CsDesk {
  const desk = useMemo(() => new CsDesk(props.session), [props.session]);
  const [, bump] = useReducer((n: number) => n + 1, 0);
  const redraw = () => bump(undefined);
  useEffect(() => {
    const off = desk.watch(redraw);
    const offSession = props.session.watch(redraw);
    void desk.fetch();
    return () => {
      off();
      offSession();
      desk.dispose();
    };
  }, [desk, props.session]);
  return desk;
}

/** The rows the core describes for one record: a pick is a list, a number a field in the row's unit, a toggle a check box, a note words. */
export function CsRows(props: { rows: CsRow[]; disabled: boolean; onEdit: (id: string, value: number | boolean) => void; prefix?: string }) {
  return (
    <div class="cs-rows">
      {props.rows.map((r) => {
        const label = props.prefix ? `${props.prefix} ${r.title}` : r.title;
        if (r.kind === "note")
          return (
            <p key={r.id} class={r.warning ? "cs-note warn" : "cs-note"}>
              {r.detail}
            </p>
          );
        return (
          <label key={`${r.id}:${r.kind}`} class="cs-row" title={r.detail || undefined}>
            <span class="cs-row-title">{r.title}</span>
            {r.kind === "pick" ? (
              <select
                aria-label={label}
                value={String(r.value ?? 0)}
                disabled={props.disabled}
                onChange={(e) => {
                  const v = Number((e.target as HTMLSelectElement).value);
                  const o = r.options?.find((x) => x.value === v);
                  if (!o || o.disabled || v === r.value) {
                    (e.target as HTMLSelectElement).value = String(r.value ?? 0);
                    return;
                  }
                  props.onEdit(r.id, v);
                }}
              >
                {(r.options ?? []).map((o) => (
                  <option key={o.value} value={String(o.value)} disabled={o.disabled}>
                    {(o.section ? `${o.section}: ` : "") + o.text + (o.owner ? ` (${o.owner})` : "")}
                  </option>
                ))}
              </select>
            ) : r.kind === "number" ? (
              <span class="cs-number">
                <input
                  type="number"
                  aria-label={label}
                  min={r.min}
                  max={r.max}
                  step={r.step && r.step > 0 ? r.step : 1}
                  value={Number((r.value ?? 0).toFixed(r.decimals ?? 0))}
                  disabled={props.disabled}
                  onChange={(e) => {
                    const v = Number((e.target as HTMLInputElement).value);
                    if (Number.isFinite(v) && v !== r.value) props.onEdit(r.id, Math.min(r.max ?? v, Math.max(r.min ?? v, v)));
                  }}
                />
                {r.unit ? <span class="cs-unit">{r.unit}</span> : null}
              </span>
            ) : (
              <input
                type="checkbox"
                aria-label={label}
                checked={r.on === true}
                disabled={props.disabled || (r.enabled === false && r.on !== true)}
                onChange={(e) => props.onEdit(r.id, (e.target as HTMLInputElement).checked)}
              />
            )}
          </label>
        );
      })}
    </div>
  );
}

/** A card's Apply row: the reason a failed apply gives, then Revert and Apply. */
function ApplyRow(props: { desk: CsDesk; rec: CsRec; ready?: boolean; hint?: string; children?: ComponentChildren }) {
  const { desk, rec } = props;
  const dirty = isDirty(rec, desk.recs);
  const ready = props.ready !== false;
  const text = rec.message ? rec.message : dirty && !ready ? (props.hint ?? "") : "";
  return (
    <div class="cs-apply">
      {props.children}
      <span class={rec.isError && rec.message ? "cs-message error" : "cs-message"} role={rec.isError ? "alert" : "status"}>
        {text}
      </span>
      <button type="button" disabled={!dirty || rec.applying} onClick={() => desk.revertRecord(rec.key)}>
        Revert
      </button>
      <button type="button" class="primary" disabled={!dirty || !ready || rec.applying || desk.busy > 0 || !desk.writable} onClick={() => void desk.apply(rec.key)}>
        Apply
      </button>
    </div>
  );
}

function placeholderOf(rec: CsRec): string {
  if (rec.kind === "binding") return typeName(num(rec.draft, "type"));
  if (rec.kind === "group") return `Group ${rec.slot + 1}`;
  if (rec.kind === "macro") return `Macro ${rec.slot + 1}`;
  return `Button ${rec.slot + 1}`;
}

function nameOfRec(desk: CsDesk, rec: CsRec): string {
  if (rec.kind === "binding") return rec.nameEdit ?? (desk.snap ? nameOf(desk.snap, rec.slot) : "");
  return typeof rec.draft.name === "string" ? rec.draft.name : "";
}

/** One card: the header (open or close, name over the summary, its state, Remove), the editor while open, and the Apply row. */
function Card(props: {
  desk: CsDesk;
  rec: CsRec;
  open: boolean;
  onOpen: (on: boolean) => void;
  summary: string;
  pill?: { text: string; good: boolean } | null;
  warning?: string;
  extras?: ComponentChildren;
  ready?: boolean;
  hint?: string;
  applyLeading?: ComponentChildren;
  children?: ComponentChildren;
}) {
  const { desk, rec } = props;
  const name = nameOfRec(desk, rec);
  const placeholder = placeholderOf(rec);
  const title = name || placeholder;
  const dirty = isDirty(rec, desk.recs);
  return (
    <section class="card cs-card" role="group" aria-label={title}>
      <div class="cs-card-head">
        <button type="button" class="cs-disclose" aria-expanded={props.open} aria-label={props.open ? `Collapse ${title}` : `Expand ${title}`} onClick={() => props.onOpen(!props.open)}>
          <span aria-hidden="true">{props.open ? "▾" : "▸"}</span>
        </button>
        <div class="cs-title">
          <input
            type="text"
            class="cs-name"
            aria-label={`Name of ${title}`}
            value={name}
            placeholder={placeholder}
            maxLength={31}
            disabled={!desk.writable}
            onInput={(e) => desk.setName(rec.key, (e.target as HTMLInputElement).value)}
          />
          <span class={rec.kind === "macro" && desk.snap && macroRunning(desk.snap) === rec.slot ? "cs-summary running" : "cs-summary"}>{props.summary}</span>
        </div>
        {props.warning ? (
          <span class="cs-warning" title={props.warning} role="img" aria-label={props.warning}>
            ⚠
          </span>
        ) : null}
        {props.pill ? <span class={props.pill.good ? "badge cs-pill good" : "badge cs-pill"}>{props.pill.text}</span> : null}
        {props.extras}
        <button
          type="button"
          class="cs-remove"
          aria-label={`Remove ${title}`}
          disabled={rec.applying || (liveConfigured(rec) && !desk.writable)}
          onClick={() => void desk.remove(rec.key)}
        >
          Remove
        </button>
      </div>
      {props.open ? <div class="cs-body">{props.children}</div> : null}
      {props.open || dirty || rec.applying || rec.message ? (
        <ApplyRow desk={desk} rec={rec} ready={props.ready} hint={props.hint}>
          {props.applyLeading}
        </ApplyRow>
      ) : null}
    </section>
  );
}

function RemoteButtons(props: { desk: CsDesk; receiver: CsRec }) {
  const { desk } = props;
  const snap = desk.snap!;
  const keys = keysFor("remotes", snap, desk.recs);
  const live = slotActive(snap, props.receiver.slot);
  return (
    <div class="cs-remotes" role="group" aria-label="Remote buttons">
      <h4>Remote buttons</h4>
      {keys.length === 0 ? <p class="empty">No remote buttons yet. Add one, then learn its code from the remote.</p> : null}
      {keys.map((k) => {
        const r = desk.rec(k)!;
        const learning = desk.learning === r.slot;
        return (
          <div class="cs-remote" key={k} role="group" aria-label={`Remote button ${r.slot + 1}`}>
            <div class="cs-remote-head">
              <span class="cs-code">{codeText(r.draft)}</span>
              {learning ? (
                <button type="button" onClick={() => void desk.cancelLearn()}>
                  Cancel
                </button>
              ) : (
                <button type="button" disabled={!desk.writable || desk.learning >= 0 || !live} title={live ? undefined : "Apply the receiver first."} onClick={() => void desk.startLearn(r.slot)}>
                  Learn
                </button>
              )}
              <button type="button" aria-label={`Remove remote button ${r.slot + 1}`} disabled={r.applying || (liveConfigured(r) && !desk.writable)} onClick={() => void desk.remove(k)}>
                Remove
              </button>
            </div>
            {learning ? <p class="cs-note" role="status">{desk.learnMessage}</p> : null}
            {r.message ? <p class={r.isError ? "cs-note error" : "cs-note"}>{r.message}</p> : null}
            <CsRows rows={rowsOf(r.description)} disabled={!desk.writable || r.applying} onEdit={(id, v) => void desk.edit(k, id, v)} prefix={`Remote button ${r.slot + 1}`} />
          </div>
        );
      })}
    </div>
  );
}

function DisplayPanel(props: { desk: CsDesk }) {
  const { desk } = props;
  const snap = desk.snap!;
  useEffect(() => {
    desk.watchDisplay(true);
    return () => desk.watchDisplay(false);
  }, [desk]);
  const panel = panelState(snap);
  const pages = activePages(snap);
  return (
    <div class="cs-display" role="group" aria-label="Display">
      <p class={panel.error ? "cs-note warn" : "cs-note"}>
        Panel: <strong class={panel.good ? "ok" : undefined}>{panel.text}</strong>. {panel.detail}
      </p>
      <h4>Display settings</h4>
      <CsRows rows={rowsOf(desk.display)} disabled={!desk.writable || desk.busy > 0} onEdit={(id, v) => void desk.editDisplay(id, v)} prefix="Display" />
      <h4>Pages</h4>
      {pages.map((p, n) => (
        <div class="cs-page" key={p.index} role="group" aria-label={`Page ${n + 1}`}>
          <div class="cs-remote-head">
            <span>
              Page {n + 1}
              {p.shown ? " (shown)" : ""}: {summaryOf(desk.pages.get(p.index) ?? null)}
            </span>
            <button type="button" aria-label={`Remove page ${n + 1}`} disabled={!desk.writable || desk.busy > 0} onClick={() => void desk.removePage(p.index)}>
              Remove
            </button>
          </div>
          <CsRows rows={rowsOf(desk.pages.get(p.index) ?? null)} disabled={!desk.writable || desk.busy > 0} onEdit={(id, v) => void desk.editPage(p.index, id, v)} prefix={`Page ${n + 1}`} />
        </div>
      ))}
      <button type="button" disabled={!desk.writable || freePage(snap) === null || desk.busy > 0} onClick={() => void desk.addPage()}>
        Add Page
      </button>
      {desk.displayMessage ? <p class="cs-note error" role="alert">{desk.displayMessage}</p> : null}
    </div>
  );
}

function BindingCard(props: { desk: CsDesk; rec: CsRec; open: boolean; onOpen: (on: boolean) => void }) {
  const { desk, rec } = props;
  const snap = desk.snap!;
  const type = num(rec.draft, "type");
  const live = liveConfigured(rec);
  const active = slotActive(snap, rec.slot);
  const name = nameOfRec(desk, rec);
  const summary = summaryOf(rec.description);
  const aux = isAux(type);
  const slotStatus = (snap.status as CsRecord | undefined)?.slotStatus;
  const health = Array.isArray(slotStatus) && typeof slotStatus[rec.slot] === "number" ? (slotStatus[rec.slot] as number) : 0;
  const auxNow = auxOf(snap, rec.slot);
  const auxLive = live && active;
  return (
    <Card
      desk={desk}
      rec={rec}
      open={props.open}
      onOpen={props.onOpen}
      summary={name ? `${typeName(type)} - ${summary}` : summary}
      pill={pillOf(rec, snap, desk.recs)}
      applyLeading={
        type === TY.Ir && props.open ? (
          <button type="button" disabled={!desk.writable || freeSlot("ir", snap, desk.recs) === null} onClick={() => void desk.addRemoteButton()}>
            Add Remote Button
          </button>
        ) : null
      }
    >
      {live && !active ? (
        <p class="cs-note warn">
          Not running: {(desk.texts[String(health)] || "failed to apply the binding").toLowerCase()} Reassign the conflicting pin, then apply.
        </p>
      ) : null}
      {aux ? (
        <div class="cs-aux">
          <label class="cs-row">
            <span class="cs-row-title">Output</span>
            <input
              type="checkbox"
              aria-label={`Output ${name || typeName(type)}`}
              checked={auxNow.on}
              disabled={!auxLive || !desk.writable}
              onChange={(e) => void desk.setAuxState(rec.slot, (e.target as HTMLInputElement).checked)}
            />
          </label>
          <p class="cs-note">{auxLive ? "Switches the pin now. Instant, and never written to flash." : "Apply the output first; the switch works once it is running."}</p>
          {type === TY.AuxPwm ? (
            <label class="cs-row">
              <span class="cs-row-title">Level</span>
              <input
                type="range"
                min={0}
                max={100}
                step={1}
                aria-label={`Level ${name || typeName(type)}`}
                value={Math.round(auxNow.level)}
                style={rangeFill(Math.round(auxNow.level), 0, 100)}
                disabled={!auxLive || !desk.writable}
                onChange={(e) => void desk.setAuxLevel(rec.slot, Number((e.target as HTMLInputElement).value))}
              />
              <span class="cs-unit">{Math.round(auxNow.level)}%</span>
            </label>
          ) : null}
        </div>
      ) : null}
      <CsRows rows={rowsOf(rec.description)} disabled={!desk.writable || rec.applying} onEdit={(id, v) => void desk.edit(rec.key, id, v)} prefix={name || typeName(type)} />
      {type === TY.Ir ? <RemoteButtons desk={desk} receiver={rec} /> : null}
      {type === TY.Display && live ? <DisplayPanel desk={desk} /> : null}
    </Card>
  );
}

function GroupCard(props: { desk: CsDesk; rec: CsRec; open: boolean; onOpen: (on: boolean) => void }) {
  const { desk, rec } = props;
  const snap = desk.snap!;
  const configured = num(rec.draft, "targetKind") !== 0 && num(rec.draft, "memberMask") !== 0;
  const statuses = ((snap.ext as CsRecord | undefined)?.groupStatus ?? []) as number[];
  const st = statuses[rec.slot] ?? 0;
  return (
    <Card
      desk={desk}
      rec={rec}
      open={props.open}
      onOpen={props.onOpen}
      summary={summaryOf(rec.description)}
      warning={st !== 0 && liveConfigured(rec) ? desk.texts[String(st)] : undefined}
      ready={configured}
      hint="Choose at least one member, then apply."
    >
      <CsRows rows={rowsOf(rec.description)} disabled={!desk.writable || rec.applying} onEdit={(id, v) => void desk.edit(rec.key, id, v)} prefix={nameOfRec(desk, rec) || placeholderOf(rec)} />
    </Card>
  );
}

function MacroCard(props: { desk: CsDesk; rec: CsRec; open: boolean; onOpen: (on: boolean) => void }) {
  const { desk, rec } = props;
  const snap = desk.snap!;
  const steps = (Array.isArray(rec.draft.steps) ? rec.draft.steps : []) as CsRecord[];
  const liveSteps = Array.isArray(rec.live.steps) ? rec.live.steps.length : 0;
  const running = macroRunning(snap) === rec.slot;
  const st = (((snap.ext as CsRecord | undefined)?.macroStatus ?? []) as number[])[rec.slot] ?? 0;
  const title = nameOfRec(desk, rec) || placeholderOf(rec);
  return (
    <Card
      desk={desk}
      rec={rec}
      open={props.open}
      onOpen={props.onOpen}
      summary={macroSummary(rec, snap)}
      warning={st !== 0 && liveSteps > 0 ? desk.texts[String(st)] : undefined}
      extras={
        <button type="button" aria-label={running ? `Stop ${title}` : `Run ${title}`} disabled={!desk.writable || (!running && liveSteps === 0)} onClick={() => void desk.fire(rec.key)}>
          {running ? "Stop" : "Run"}
        </button>
      }
    >
      {steps.length === 0 ? <p class="empty">No steps yet.</p> : null}
      <ol class="cs-steps">
        {steps.map((_, i) => (
          <li key={i} class="cs-step" aria-label={`Step ${i + 1}`}>
            <div class="cs-remote-head">
              <span>
                Step {i + 1}: {summaryOf(rec.steps[i] ?? null)}
              </span>
              <button type="button" aria-label={`Move step ${i + 1} up`} disabled={i === 0 || !desk.writable} onClick={() => desk.moveStep(rec.key, i, -1)}>
                ↑
              </button>
              <button type="button" aria-label={`Move step ${i + 1} down`} disabled={i === steps.length - 1 || !desk.writable} onClick={() => desk.moveStep(rec.key, i, 1)}>
                ↓
              </button>
              <button type="button" aria-label={`Remove step ${i + 1}`} disabled={!desk.writable} onClick={() => desk.removeStep(rec.key, i)}>
                Remove
              </button>
            </div>
            <CsRows rows={rowsOf(rec.steps[i] ?? null)} disabled={!desk.writable || rec.applying} onEdit={(id, v) => void desk.editStep(rec.key, i, id, v)} prefix={`${title} step ${i + 1}`} />
          </li>
        ))}
      </ol>
      <button type="button" disabled={!desk.writable || steps.length >= maxSteps(snap)} onClick={() => void desk.addStep(rec.key)}>
        Add Step
      </button>
    </Card>
  );
}

const ADD_WORDS: Record<CsPage, string> = { controls: "Add Control", groups: "Add Group", macros: "Add Macro", aux: "Add Output" };
const EMPTY: Record<CsPage, { title: string; text: string }> = {
  controls: {
    title: "No Controls Configured",
    text: "Wire a button, switch, knob, encoder, or LED to a spare GPIO and bind it to a device function.",
  },
  groups: { title: "No Groups", text: "A group lets one control move several channels or outputs at once." },
  macros: { title: "No Macros", text: "A macro runs several device actions in order from one button, remote key or fire." },
  aux: { title: "No Auxiliary Outputs", text: "Switch a relay, an amplifier trigger or a light from a spare GPIO, at boot or from a control." },
};

const HELP =
  "Wire push buttons, toggle switches, potentiometers, rotary encoders, indicator LEDs, and an IR remote receiver to spare GPIOs and bind each to a device function. Buttons and switches wire between the GPIO and GND (internal pull-up); pots use an ADC pin (GPIO 26, 27, or 28) between 3V3 and GND; encoders use two GPIOs with the common wired to GND. LEDs drive active-high by default. An IR receiver module's OUT pin connects to any GPIO, and its remote buttons are learned by pressing them at the device.\n\nThis wiring is a board-level setting: it is stored on the device, survives preset changes, and survives a factory reset. Changes take effect immediately as a live preview; use Save to keep them across a reboot, or Revert to discard them.";

export function SurfacesScreen(props: ScreenProps) {
  const desk = useDesk(props);
  const snap = desk.snap;
  const pages = pagesOf(snap);
  const [page, setPage] = useState<CsPage>("controls");
  const shown = pages.includes(page) ? page : "controls";
  const [opened, setOpened] = useState<Record<string, boolean>>({});
  const setOpen = (key: string, on: boolean) => setOpened((o) => ({ ...o, [key]: on }));
  const [addType, setAddType] = useState(0);
  const [saveMessage, setSaveMessage] = useState("");
  const readOnly = props.session.readOnlyReason;

  if (!desk.loaded || !snap)
    return (
      <div class="surfaces-screen">
        <section class="card">
          {desk.loadError ? (
            <p class="error" role="alert">
              {desk.loadError}{" "}
              <button type="button" onClick={() => void desk.fetch()}>
                Try Again
              </button>
            </p>
          ) : (
            <p class="empty" role="status">
              Reading control-surface capabilities from the device…
            </p>
          )}
        </section>
      </div>
    );
  if (!supported(snap))
    return (
      <div class="surfaces-screen">
        <section class="card">
          <p class="empty">This firmware has no control surfaces.</p>
        </section>
      </div>
    );

  const keys = keysFor(shown, snap, desk.recs);
  const kind = shown === "groups" ? "group" : shown === "macros" ? "macro" : "binding";
  const free = freeSlot(kind, snap, desk.recs);
  const types = shown === "controls" || shown === "aux" ? addableTypes(shown, snap, desk.recs) : [];
  const chosenType = types.find((t) => t.type === addType)?.type ?? types[0]?.type ?? 0;
  const add = async () => {
    let key: string | null = null;
    if (shown === "groups") key = desk.addGroup();
    else if (shown === "macros") key = desk.addMacro();
    else if (chosenType) key = await desk.addControl(chosenType);
    if (key) setOpen(key, true);
  };
  const dirty = deviceDirty(snap);
  const anyDraft = [...desk.recs.values()].some((r) => isDirty(r, desk.recs));

  return (
    <div class="surfaces-screen">
      {readOnly ? (
        <div class="banner" role="status">
          {readOnly}
        </div>
      ) : null}
      <section class="card cs-save" aria-label="Control surfaces">
        <div class="card-head">
          <h3>Control Surfaces</h3>
          <span class={dirty ? "badge cs-pill" : "badge cs-pill good"} role="status">
            {dirty ? "Unsaved changes" : "Saved"}
          </span>
        </div>
        <p class="cs-note">
          {dirty ? "Changes run now as a live preview. Save keeps them across a reboot; Revert puts back what is stored." : "What runs now is what is stored on the device."}
          {anyDraft ? " Cards marked Pending have edits that are not applied yet." : ""}
        </p>
        <div class="cs-apply">
          <span class={saveMessage ? "cs-message error" : "cs-message"} role={saveMessage ? "alert" : undefined}>
            {saveMessage}
          </span>
          <button
            type="button"
            disabled={!dirty || !desk.writable || desk.busy > 0}
            onClick={async () => {
              const a = await props.ask({
                title: "Revert Control Surfaces",
                body: <p>Put back the control-surface configuration stored on the device? Changes not saved are lost.</p>,
                buttons: [
                  { label: "Revert", value: "yes", danger: true },
                  { label: "Cancel", value: "cancel" },
                ],
              });
              if (a === "yes") setSaveMessage(await desk.revert());
            }}
          >
            Revert
          </button>
          <button type="button" class="primary" disabled={!dirty || !desk.writable || desk.busy > 0} onClick={async () => setSaveMessage(await desk.save())}>
            Save
          </button>
        </div>
      </section>
      {pages.length > 1 ? (
        <div class="screen-tabs cs-tabs" role="tablist" aria-label="Control-surface pages">
          {pages.map((p) => (
            <button type="button" role="tab" key={p} aria-selected={p === shown} onClick={() => setPage(p)}>
              {PAGE_LABELS[p]}
            </button>
          ))}
        </div>
      ) : null}
      {keys.length === 0 ? (
        <section class="card cs-empty">
          <h3>{EMPTY[shown].title}</h3>
          <p class="empty">{EMPTY[shown].text}</p>
        </section>
      ) : null}
      {keys.map((k) => {
        const rec = desk.rec(k)!;
        const open = opened[k] === true;
        const onOpen = (on: boolean) => setOpen(k, on);
        if (rec.kind === "group") return <GroupCard key={k} desk={desk} rec={rec} open={open} onOpen={onOpen} />;
        if (rec.kind === "macro") return <MacroCard key={k} desk={desk} rec={rec} open={open} onOpen={onOpen} />;
        return <BindingCard key={k} desk={desk} rec={rec} open={open} onOpen={onOpen} />;
      })}
      <div class="cs-add">
        {types.length > 1 ? (
          <select aria-label="Component type" value={String(chosenType)} disabled={!desk.writable} onChange={(e) => setAddType(Number((e.target as HTMLSelectElement).value))}>
            {types.map((t) => (
              <option key={t.type} value={String(t.type)}>
                {t.name}
              </option>
            ))}
          </select>
        ) : null}
        <button type="button" disabled={!desk.writable || free === null || ((shown === "controls" || shown === "aux") && !chosenType) || desk.busy > 0} onClick={() => void add()}>
          {ADD_WORDS[shown]}
          {(shown === "controls" || shown === "aux") && types.length === 1 ? `: ${types[0]!.name}` : ""}
        </button>
        {free === null && desk.writable && kind === "binding" ? <span class="empty">All {slotCount(snap)} control slots are in use.</span> : null}
      </div>
      {shown === "controls" ? (
        <section class="card cs-help">
          {HELP.split("\n\n").map((p, i) => (
            <p key={i} class="cs-note">
              {p}
            </p>
          ))}
          {num(snap, "capsVersion") ? <p class="cs-note">Control-surface capability version {num(snap, "capsVersion")}.</p> : null}
        </section>
      ) : null}
    </div>
  );
}
