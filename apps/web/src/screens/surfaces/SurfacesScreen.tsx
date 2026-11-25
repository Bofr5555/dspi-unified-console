// WP WB6l: the control-surface editor (CG20), after the Console's
// ControlSurfacesSettingsTab and Qt's Settings > Control Surfaces, Groups,
// Macros and Auxiliary Outputs (WP QP12). One card per wired component, group
// or macro, collapsed to its summary; a card's rows are the ones the core
// describes for its draft (`csDescribe`), every change is the core's next draft
// (`csEdit`) and nothing reaches the device until Apply (`csApply`). The device
// runs an applied change at once as a live preview; Save writes it to flash and
// Revert reloads what is stored. Read only: everything shows, nothing can be
// changed, and the core refuses a write that got past the page.
//
// -- WP WEB7 -- Settings › Control as Qt Modern has it: no screen of its own; the
// Settings screen lists Control Surfaces, Channel Groups, Macros and Auxiliary Outputs
// beside the page and shows one (`SurfacesPage`). Cards as Qt's CsCard (chevron, the
// type's tinted badge, the name over the summary, the state chip, a trash button);
// "Add Control" a menu of the types (no select); Save and Revert on a bar along the
// bottom only while something is unsaved (`SurfacesSaveBar`, Qt's SettingsSaveBar).

import type { ComponentChildren } from "preact";
import { useEffect, useMemo, useReducer, useRef, useState } from "preact/hooks";
import type { CsRecord, CsRow } from "../../core/protocol";
import type { Ask } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { CsDesk } from "./desk";
import { EmptyState, rangeFill } from "../../ui/design"; // -- WP WEB2 --
import { Group } from "../../ui/Group"; // -- WP WEB7 --
import { Icon } from "../../ui/icons"; // -- WP WEB7 --
import "./surfaces.css"; // -- WP WEB7 --
import {
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

/**
 * -- WP WEB7 -- The control-surface desk of a session's Settings: made with the screen, read when
 * `active` turns on (the Control part shows); a learn waiting for a press ends when it goes.
 */
export function useCsDesk(session: Session, active: boolean): CsDesk {
  const desk = useMemo(() => new CsDesk(session), [session]);
  const [, bump] = useReducer((n: number) => n + 1, 0);
  useEffect(() => {
    const redraw = () => bump(undefined);
    const off = desk.watch(redraw);
    const offSession = session.watch(redraw);
    return () => {
      off();
      offSession();
      desk.dispose();
    };
  }, [desk, session]);
  useEffect(() => {
    if (active) void desk.fetch();
  }, [desk, active]);
  return desk;
}

/** -- WP WEB7 -- The editor's pages this device has (Controls always; the rest by its capabilities). */
export function csPagesOf(desk: CsDesk): CsPage[] {
  return desk.loaded && desk.snap ? pagesOf(desk.snap) : ["controls"];
}

// -- WP WEB7 -- Qt's csTypeTints (Theme.qml) and CsController.typeIcon, by component type
const TYPE_TINTS = ["#75879e", "#339ebd", "#267d9e", "#367dd1", "#575ecc", "#eda12e", "#e67333", "#8c59b8", "#04856f", "#7a5aac", "#9e669e"];
const TYPE_ICONS: Record<number, string> = {
  [TY.Button]: "pointer",
  [TY.Switch]: "toggle-left",
  [TY.Pot]: "gauge",
  [TY.Encoder]: "rotate-cw",
  [TY.Led]: "lightbulb",
  [TY.LedPwm]: "sun",
  [TY.Ir]: "radio",
  [TY.Display]: "monitor",
  [TY.AuxOut]: "power",
  [TY.AuxPwm]: "sun-dim",
};

/** A component type's badge (Qt's CsBadge): a rounded square in the type's tint with its glyph in white. */
function TypeBadge(props: { type: number }) {
  const t = props.type;
  return (
    <span class="cs-badge" style={{ background: t > 0 && t < TYPE_TINTS.length ? TYPE_TINTS[t] : TYPE_TINTS[0] }} role="img" aria-label={typeName(t)}>
      <Icon name={TYPE_ICONS[t] ?? "gauge"} />
    </span>
  );
}

/** A quiet icon button (Qt's IconButton): the icon alone, its name for assistive technology and as a tooltip. */
function IconButton(props: { icon: string; label: string; title?: string; disabled?: boolean; class?: string; onClick: () => void }) {
  return (
    <button type="button" class={`cs-icon-button${props.class ? ` ${props.class}` : ""}`} aria-label={props.label} title={props.title ?? props.label} disabled={props.disabled} onClick={props.onClick}>
      <Icon name={props.icon} />
    </button>
  );
}

/** "+ Add Control" (Qt's CsAddButton): a menu of the component types it can add. ARIA's menu button pattern. */
function AddMenu(props: { label: string; types: { type: number; name: string }[]; disabled?: boolean; primary?: boolean; onAdd: (type: number) => void }) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const items = () => Array.from(list.current?.querySelectorAll<HTMLButtonElement>("[role=menuitem]:not([disabled])") ?? []);
  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  };
  useEffect(() => {
    if (!open) return;
    items()[0]?.focus();
    const outside = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!list.current?.contains(t) && !button.current?.contains(t)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);
  const onKey = (e: KeyboardEvent) => {
    if (!open) return;
    const all = items();
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const go = (i: number) => all[(i + all.length) % all.length]?.focus();
    if (e.key === "ArrowDown") go(at + 1);
    else if (e.key === "ArrowUp") go(at - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(all.length - 1);
    else if (e.key === "Escape") close(true);
    else if (e.key === "Tab") return setOpen(false);
    else return;
    e.preventDefault();
  };
  return (
    <div class="menu cs-add-menu" onKeyDown={onKey}>
      <button ref={button} type="button" class={props.primary ? "primary cs-add-button" : "cs-add-button"} aria-haspopup="menu" aria-expanded={open} disabled={props.disabled} onClick={() => setOpen(!open)}>
        <Icon name="plus" />
        {props.label}
      </button>
      {open ? (
        <ul ref={list} role="menu" aria-label={props.label}>
          {props.types.map((t) => (
            <li key={t.type} role="none">
              <button
                type="button"
                role="menuitem"
                tabIndex={-1}
                onClick={() => {
                  close(true);
                  props.onAdd(t.type);
                }}
              >
                {t.name}
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

/** A plain "+ Add Group" / "+ Add Macro" button (Qt's AppButton with the plus icon). */
function AddButton(props: { label: string; disabled?: boolean; primary?: boolean; onClick: () => void }) {
  return (
    <button type="button" class={props.primary ? "primary cs-add-button" : "cs-add-button"} disabled={props.disabled} onClick={props.onClick}>
      <Icon name="plus" />
      {props.label}
    </button>
  );
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
      {/* -- WP WEB7 -- the accent only when there is something to apply (no blue button on a clean card) */}
      <button
        type="button"
        class={dirty && ready ? "primary" : undefined}
        disabled={!dirty || !ready || rec.applying || desk.busy > 0 || !desk.writable}
        onClick={() => void desk.apply(rec.key)}
      >
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
  /** -- WP WEB7 -- the type's badge (a binding), else this icon (a group, a macro) */
  badgeType?: number;
  icon?: string;
  iconRunning?: boolean;
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
        <button type="button" class="cs-icon-button cs-disclose" aria-expanded={props.open} aria-label={props.open ? `Collapse ${title}` : `Expand ${title}`} title={props.open ? "Collapse" : "Expand"} onClick={() => props.onOpen(!props.open)}>
          <Icon name={props.open ? "chevron-down" : "chevron-right"} />
        </button>
        {/* -- WP WEB7 -- Qt's badge or icon */}
        {props.badgeType !== undefined ? (
          <TypeBadge type={props.badgeType} />
        ) : props.icon ? (
          <span class={props.iconRunning ? "cs-card-icon running" : "cs-card-icon"} aria-hidden="true">
            <Icon name={props.icon} />
          </span>
        ) : null}
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
            <Icon name="circle-alert" />
          </span>
        ) : null}
        {/* -- WP WEB7 -- the state as a status chip (§13): a dot and the word */}
        {props.pill ? (
          <span class={`status-chip cs-pill${props.pill.good ? " good status-chip-ok" : " status-chip-idle"}`}>
            <span class={`status-dot${props.pill.good ? " ok" : ""}`} aria-hidden="true" />
            {props.pill.text}
          </span>
        ) : null}
        {props.extras}
        <IconButton icon="trash" class="cs-remove" label={`Remove ${title}`} title="Remove" disabled={rec.applying || (liveConfigured(rec) && !desk.writable)} onClick={() => void desk.remove(rec.key)} />
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
              <IconButton icon="trash" label={`Remove remote button ${r.slot + 1}`} title="Remove" disabled={r.applying || (liveConfigured(r) && !desk.writable)} onClick={() => void desk.remove(k)} />
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
            <IconButton icon="trash" label={`Remove page ${n + 1}`} title="Remove" disabled={!desk.writable || desk.busy > 0} onClick={() => void desk.removePage(p.index)} />
          </div>
          <CsRows rows={rowsOf(desk.pages.get(p.index) ?? null)} disabled={!desk.writable || desk.busy > 0} onEdit={(id, v) => void desk.editPage(p.index, id, v)} prefix={`Page ${n + 1}`} />
        </div>
      ))}
      <AddButton label="Add Page" disabled={!desk.writable || freePage(snap) === null || desk.busy > 0} onClick={() => void desk.addPage()} />
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
      badgeType={type}
      applyLeading={
        type === TY.Ir && props.open ? (
          <AddButton label="Add Remote Button" disabled={!desk.writable || freeSlot("ir", snap, desk.recs) === null} onClick={() => void desk.addRemoteButton()} />
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
      icon="group"
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
      icon={running ? "play" : "list-ordered"}
      iconRunning={running}
      extras={
        <IconButton
          icon={running ? "square" : "play"}
          label={running ? `Stop ${title}` : `Run ${title}`}
          title={running ? "Stop this macro" : "Run this macro now"}
          disabled={!desk.writable || (!running && liveSteps === 0)}
          onClick={() => void desk.fire(rec.key)}
        />
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
              <IconButton icon="arrow-up" label={`Move step ${i + 1} up`} title="Move Up" disabled={i === 0 || !desk.writable} onClick={() => desk.moveStep(rec.key, i, -1)} />
              <IconButton icon="arrow-down" label={`Move step ${i + 1} down`} title="Move Down" disabled={i === steps.length - 1 || !desk.writable} onClick={() => desk.moveStep(rec.key, i, 1)} />
              <IconButton icon="circle-minus" label={`Remove step ${i + 1}`} title="Remove Step" disabled={!desk.writable} onClick={() => desk.removeStep(rec.key, i)} />
            </div>
            <CsRows rows={rowsOf(rec.steps[i] ?? null)} disabled={!desk.writable || rec.applying} onEdit={(id, v) => void desk.editStep(rec.key, i, id, v)} prefix={`${title} step ${i + 1}`} />
          </li>
        ))}
      </ol>
      <AddButton label="Add Step" disabled={!desk.writable || steps.length >= maxSteps(snap)} onClick={() => void desk.addStep(rec.key)} />
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

/** -- WP WEB7 -- One page of Settings › Control: its cards, Add, and on Control Surfaces the wiring help. */
export function SurfacesPage(props: { desk: CsDesk; page: CsPage; session: Session }) {
  const { desk } = props;
  const snap = desk.snap;
  const pages = pagesOf(snap);
  const shown = pages.includes(props.page) ? props.page : "controls";
  const [opened, setOpened] = useState<Record<string, boolean>>({});
  const setOpen = (key: string, on: boolean) => setOpened((o) => ({ ...o, [key]: on }));
  const readOnly = props.session.readOnlyReason;

  if (!desk.loaded || !snap)
    return desk.loadError ? (
      <p class="error" role="alert">
        {desk.loadError}{" "}
        <button type="button" onClick={() => void desk.fetch()}>
          Try Again
        </button>
      </p>
    ) : (
      <p class="cs-note" role="status">
        Reading control-surface capabilities from the device…
      </p>
    );
  if (!supported(snap)) return <EmptyState icon="circle-slash" title="No Control Surfaces" text="This firmware has no control surfaces." />;

  const keys = keysFor(shown, snap, desk.recs);
  const kind = shown === "groups" ? "group" : shown === "macros" ? "macro" : "binding";
  const free = freeSlot(kind, snap, desk.recs);
  const types = shown === "controls" || shown === "aux" ? addableTypes(shown, snap, desk.recs) : [];
  const add = async (type: number) => {
    let key: string | null = null;
    if (shown === "groups") key = desk.addGroup();
    else if (shown === "macros") key = desk.addMacro();
    else if (type) key = await desk.addControl(type);
    if (key) setOpen(key, true);
  };
  const addOff = !desk.writable || free === null || desk.busy > 0;
  const adder = (primary: boolean) =>
    shown === "controls" || shown === "aux" ? (
      <AddMenu label={ADD_WORDS[shown]} types={types} primary={primary} disabled={addOff || types.length === 0} onAdd={(t) => void add(t)} />
    ) : (
      <AddButton label={ADD_WORDS[shown]} primary={primary} disabled={addOff} onClick={() => void add(0)} />
    );

  return (
    <div class="surfaces-screen">
      {readOnly ? (
        <div class="banner" role="status">
          {readOnly}
        </div>
      ) : null}
      {keys.length === 0 ? (
        // -- WP WEB2 -- (§16) nothing on this page yet: the empty state, not a card with a sentence
        // -- WP WEB7 -- with its Add as the primary next step (Qt's CsEmptyState)
        <EmptyState icon="list-ordered" title={EMPTY[shown].title} text={EMPTY[shown].text}>
          {adder(true)}
        </EmptyState>
      ) : null}
      {keys.map((k) => {
        const rec = desk.rec(k)!;
        const open = opened[k] === true;
        const onOpen = (on: boolean) => setOpen(k, on);
        if (rec.kind === "group") return <GroupCard key={k} desk={desk} rec={rec} open={open} onOpen={onOpen} />;
        if (rec.kind === "macro") return <MacroCard key={k} desk={desk} rec={rec} open={open} onOpen={onOpen} />;
        return <BindingCard key={k} desk={desk} rec={rec} open={open} onOpen={onOpen} />;
      })}
      {keys.length ? (
        <div class="cs-add">
          {adder(false)}
          {free === null && desk.writable && kind === "binding" ? <span class="cs-note">All {slotCount(snap)} control slots are in use.</span> : null}
        </div>
      ) : null}
      {shown === "controls" ? (
        <Group class="cs-help" footer={num(snap, "capsVersion") ? `Control-surface capability version ${num(snap, "capsVersion")}.` : undefined}>
          {HELP.split("\n\n").map((p, i) => (
            <p key={i} class="cs-note">
              {p}
            </p>
          ))}
        </Group>
      ) : null}
    </div>
  );
}

/**
 * -- WP WEB7 -- Qt's SettingsSaveBar for the control surfaces: along the bottom of the page, only
 * while the device runs a change it has not stored (or a save failed): one Revert, one Save.
 */
export function SurfacesSaveBar(props: { desk: CsDesk; ask: Ask }) {
  const { desk } = props;
  const [message, setMessage] = useState("");
  const snap = desk.snap;
  const dirty = !!snap && desk.loaded && supported(snap) && deviceDirty(snap);
  if (!dirty && !message) return null;
  return (
    <div class="cs-save-bar" role="region" aria-label="Control surfaces">
      <Icon name="circle-alert" class="cs-save-icon" />
      <div class="cs-save-text">
        {dirty ? (
          <>
            <span class="cs-save-title" role="status">
              Unsaved changes
            </span>
            <span class="cs-save-sub">Your controls are live now; saving keeps them across a reboot.</span>
          </>
        ) : null}
        {message ? (
          <span class="cs-save-failure" role="alert">
            {message}
          </span>
        ) : null}
      </div>
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
          if (a === "yes") setMessage(await desk.revert());
        }}
      >
        Revert
      </button>
      <button type="button" class="primary" disabled={!dirty || !desk.writable || desk.busy > 0} onClick={async () => setMessage(await desk.save())}>
        Save
      </button>
    </div>
  );
}
