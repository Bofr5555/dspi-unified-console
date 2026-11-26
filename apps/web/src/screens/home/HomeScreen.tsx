// -- WP UB2 -- (UXP B2, D4) Home: a remote for the daily listener, Simple mode's first page and
// the locked user's only page (UI-MODES section 2). The device and its connection, one big
// User volume with Mute, the source as segments, the presets as a Recall list, the rooms' and
// zones' volumes on an installation, one health line; nothing else. Nothing here is refused:
// the installer's lock lets volume, mute, the source (-- FIX016 -- LOCKSRC, owner D4) and preset
// loads through, and Save, Copy Here and the Master volume are not on the page at all (the Master
// is the Installer's; Library › Presets and the bar's popover keep the rest).

import { useCallback, useEffect, useState } from "preact/hooks";
import { useSessionState } from "../../ui/components";
import { Glyph, Segments, rangeFill } from "../../ui/design";
import type { Ask, ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { useHealth } from "../../ui/status-rail/HealthRail";
import { healthSummary } from "../../ui/status-rail/health";
import { dragProps, useLiveWrite } from "../main/live";
import { USER_STEPS, posToUser, userToPos, volumeText } from "../main/model";
import { dockSources } from "../../ui/shell/Dock";
import { useSourceNames } from "../inputs/InputsScreen";
import { sourceTitle } from "../inputs/model";
import { connectionWord } from "../system/StatusRail";
import { useFeed } from "../system/SystemScreen";
import type { SystemFeed } from "../system/feed";
import type { SystemModel } from "../system/model";
import { cleanText } from "../system/installation/model";
import type { SystemVolumePlan } from "../system/volume-protocol";
import { healthLine, recallRows, roomLevel, sourceChoosable } from "./model";
import "./home.css";

export function HomeScreen(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const snap = st.snapshot;
  const names = useSourceNames(session).names ?? [];
  const live = useLiveWrite((db) => session.write("vol.user", { float: db }), session);
  const [busy, setBusy] = useState(false);
  if (!snap) return null;
  const s = snap.state;
  const locked = props.locked === true;
  const off = session.readOnlyReason !== "";
  const db = live.draft ?? s.userVolumeDb;
  const pos = userToPos(db);
  const sources = dockSources(s);
  const where = [st.place?.unit, st.place?.location].filter((x) => !!x).join(" · ");
  const word = connectionWord(st);

  const recall = async (slot: number) => {
    if (slot === s.activePresetSlot || busy) return;
    setBusy(true);
    try {
      // The locked user has nothing to save (only volume, mute, the source and presets change): a plain load
      if (!locked) {
        const a = await props.unsaved();
        if (a === "cancel") return;
        if (a === "save" && !(await session.savePreset(s.activePresetSlot))) return;
      }
      await session.write("preset.load", "trigger", [slot]);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="home-screen">
      <header class="home-head">
        <h2 class="home-name">{snap.device.name || "DSPi"}</h2>
        {where ? <span class="home-where">{where}</span> : null}
        <span class={`home-link${word === "Connected" ? " ok" : ""}`} role="status">
          <span class="home-dot" aria-hidden="true" />
          {word}
        </span>
      </header>

      <section class="card home-volume" aria-label="Volume">
        <div class="home-volume-row">
          <Glyph name={s.userMute ? "volume-x" : "volume"} class="home-volume-icon" />
          <input
            type="range"
            class="home-slider"
            aria-label="Volume"
            aria-valuetext={volumeText(db, false)}
            min={0}
            max={USER_STEPS}
            step={1}
            value={pos}
            style={rangeFill(pos, 0, USER_STEPS)}
            disabled={off}
            {...dragProps(live)}
            onInput={(e) => live.move(posToUser(Number((e.target as HTMLInputElement).value)))}
            onChange={(e) => live.release(posToUser(Number((e.target as HTMLInputElement).value)))}
          />
          <output class="home-volume-value" aria-live="polite">
            {volumeText(db, false)}
          </output>
        </div>
        <div class="home-volume-row">
          <button type="button" class="toggle home-mute" aria-pressed={s.userMute} disabled={off} onClick={() => void session.write("vol.mute", { bool: !s.userMute })}>
            <Glyph name="volume-x" />
            Mute
          </button>
          {sourceChoosable(sources.length) ? (
            <Segments
              label="Source"
              options={sources.map((x) => ({ value: x, text: sourceTitle(s, x, names) }))}
              value={s.inputSource}
              disabled={off}
              choose={(to) => void session.write("in.source", { choice: to })}
            />
          ) : (
            <span class="home-source">
              Source: {sourceTitle(s, s.inputSource, names)}
            </span>
          )}
        </div>
      </section>

      <section class="card home-presets" aria-label="Presets">
        <h3 class="section-label">Presets</h3>
        <ul class="home-recall" role="list">
          {recallRows(s).map((r) => (
            <li key={r.slot}>
              <button
                type="button"
                class="home-recall-row"
                aria-pressed={r.active}
                aria-label={`Recall preset ${r.slot + 1}: ${r.text}`}
                title={r.active ? "Playing now" : "Recall this preset"}
                disabled={off || busy}
                onClick={() => void recall(r.slot)}
              >
                <span class="home-recall-mark" aria-hidden="true">
                  {r.active ? "▸" : ""}
                </span>
                {r.text}
              </button>
            </li>
          ))}
        </ul>
      </section>

      {props.system ? <Rooms feed={props.system} session={session} ask={props.ask} /> : null}

      {props.health ? <HealthLine {...props} /> : null}
    </div>
  );
}

function HealthLine(props: ScreenProps) {
  const view = useHealth(props.health!);
  const summary = healthSummary(view.problems);
  return (
    <p class={`home-health health-${summary.tone}`} role="status">
      Health: {healthLine(summary)}
    </p>
  );
}

/** The rooms and zones of the session's installation, when the hub lets this user set their volume. */
function Rooms(props: { feed: SystemFeed; session: Session; ask: Ask }) {
  const st = useFeed(props.feed);
  const model = st.kind === "hub" ? st.model : null;
  const may = !props.feed.readOnly && (st.role === "admin" || st.role === "control");
  if (!model || !may || (!model.rooms.length && !model.zones.length)) return null;
  return (
    <section class="card home-rooms" aria-label="Rooms">
      <h3 class="section-label">Rooms</h3>
      <ul class="home-room-list" role="list">
        {targetsOf(model).map((t) => (
          <RoomRow key={t.id} target={t} session={props.session} ask={props.ask} />
        ))}
      </ul>
    </section>
  );
}

interface Target {
  id: string;
  kind: "room" | "zone";
  key: string;
  name: string;
}

function targetsOf(model: SystemModel): Target[] {
  return [
    ...model.rooms.map((r) => ({ id: `room:${r.id}`, kind: "room" as const, key: r.id, name: cleanText(r.name || r.id) })),
    ...model.zones.map((z) => ({ id: `zone:${z.id}`, kind: "zone" as const, key: z.id, name: cleanText(z.name || z.id) })),
  ];
}

/** Step of a room's − and + (dB of the user volume). */
const ROOM_STEP_DB = 3;

/**
 * One room or zone: its level as its units have it, − and + in 3 dB steps. A step that makes a
 * unit louder asks first, as System › Units does (FEATUI); a refused unit says why.
 */
function RoomRow(props: { target: Target; session: Session; ask: Ask }) {
  const { target, session } = props;
  const [level, setLevel] = useState("–");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const plan = useCallback(
    (stepDb: number) => session.core.call("systemVolumePlan", { [target.kind]: target.key, volume: "user", stepDb }),
    [session, target.kind, target.key],
  );
  const read = useCallback(async () => {
    try {
      const p = await plan(0);
      setLevel(roomLevel(p.units.map((u) => u.beforeDb)));
    } catch {
      setLevel("–");
    }
  }, [plan]);
  useEffect(() => void read(), [read]);
  const step = async (by: number) => {
    setBusy(true);
    setMessage("");
    try {
      const p: SystemVolumePlan = await plan(by);
      const louder = p.units.filter((u) => u.state === "ready" && u.louder);
      if (louder.length) {
        const go = await props.ask({
          title: "Make it louder?",
          body: <p>{louder.length === 1 ? `1 unit of ${cleanText(p.name)} gets louder.` : `${louder.length} units of ${cleanText(p.name)} get louder.`}</p>,
          buttons: [
            { label: "Set", value: "set" },
            { label: "Cancel", value: "cancel" },
          ],
        });
        if (go !== "set") return;
      }
      const done = await session.core.call("systemVolumeSet", { plan: p });
      setLevel(roomLevel(done.units.map((u) => u.afterDb ?? u.beforeDb)));
      const refused = done.units.filter((u) => u.state === "refused");
      if (refused.length) setMessage(refused.map((u) => `${cleanText(u.name)}: ${cleanText(u.message)}`).join(" "));
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <li class="home-room">
      <span class="home-room-name">
        {target.kind === "zone" ? "Zone " : ""}
        {target.name}
      </span>
      <output class="home-room-level">{level}</output>
      <button type="button" aria-label={`${target.name}: quieter`} disabled={busy} onClick={() => void step(-ROOM_STEP_DB)}>
        −
      </button>
      <button type="button" aria-label={`${target.name}: louder`} disabled={busy} onClick={() => void step(ROOM_STEP_DB)}>
        +
      </button>
      {message ? <p class="note home-room-note">{message}</p> : null}
    </li>
  );
}
