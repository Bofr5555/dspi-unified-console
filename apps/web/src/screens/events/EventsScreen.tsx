// The event log (WP WB8): what the core recorded for the devices this session
// has seen (P0M: clips, the audio link's lock and loss, resets, connections,
// temperature and counters) and, on a hub's installation, its alarms; each
// with its time, unit and text, newest first. Filter by unit; copy as text.
// Read every second while shown and the page is visible, only what is new.
//
// And Save Diagnostics… (Advanced and Installer): the core's bundle, what
// `uconsole diag` and the desktop's Help > Save Diagnostics… write, handed to
// the viewer as a file to save. Nothing is sent anywhere.

import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type { EventRecord } from "../../core/protocol";
import type { ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { useFeed } from "../system/SystemScreen";
import { alarmRow, fullTime, merged, recordRow, rowsText, shownRows, timeText, unitsOf, type EventRow } from "./model";

/** How often the log is read while shown. */
export const EVENTS_POLL_MS = 1000;

/** The core's records, every device's, kept current while mounted. */
function useEventLog(session: Session, pollMs = EVENTS_POLL_MS): { records: EventRecord[]; error: string } {
  const [records, setRecords] = useState<EventRecord[]>([]);
  const [error, setError] = useState("");
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const after = new Map<string, number>();
    let kept: EventRecord[] = [];
    const read = async () => {
      const visible = typeof document === "undefined" || document.visibilityState === "visible";
      if (visible) {
        try {
          const serials = await session.core.call("eventLogDevices", undefined);
          const fresh: EventRecord[] = [];
          for (const serial of serials) {
            const r = await session.core.call("eventLogRead", { serial, afterId: after.get(serial) ?? 0 });
            if (r.lastId) after.set(serial, r.lastId);
            for (const rec of r.records) fresh.push(rec.serial ? rec : { ...rec, serial });
          }
          if (!alive) return;
          if (fresh.length) {
            kept = merged(kept, fresh);
            setRecords(kept);
          }
          setError("");
        } catch (e) {
          if (alive) setError(e instanceof Error ? e.message : String(e));
        }
      }
      if (alive) timer = setTimeout(() => void read(), pollMs);
    };
    void read();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [session, pollMs]);
  return { records, error };
}

export function EventsScreen(props: ScreenProps) {
  const { session } = props;
  const log = useEventLog(session);
  const feed = useFeed(props.system!);
  const snap = session.state.snapshot;
  const [unit, setUnit] = useState("");
  const [copied, setCopied] = useState("");

  const rows = useMemo(() => {
    const own = snap?.device;
    const unitName = (serial: string) => {
      const u = feed.model?.units.find((x) => x.serial === serial) ?? feed.model?.senders.find((x) => x.serial === serial);
      if (u?.name) return u.name;
      if (own && serial === own.serial) return own.name || serial;
      return serial;
    };
    const out: EventRow[] = log.records.map((r) => recordRow(r, unitName));
    if (feed.kind === "hub") for (const a of feed.status?.alarms ?? []) out.push(alarmRow(a, feed.model));
    return out;
  }, [log.records, feed.kind, feed.model, feed.status?.alarms, snap?.device]);
  const units = unitsOf(rows);
  const shown = shownRows(rows, unit);
  const now = new Date();

  const copy = async () => {
    const text = rowsText(shown);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(shown.length === 1 ? "1 event copied." : `${shown.length} events copied.`);
    } catch {
      setCopied("The browser did not allow copying here.");
    }
  };

  return (
    <section class="card events" aria-label="Event log">
      <div class="card-head events-head">
        <h2>Event log</h2>
        <label class="events-unit">
          Unit
          <select value={unit} onChange={(e) => setUnit((e.target as HTMLSelectElement).value)}>
            <option value="">Every unit</option>
            {units.map((u) => (
              <option key={u.id} value={u.id}>
                {u.name}
              </option>
            ))}
          </select>
        </label>
        <button type="button" disabled={!shown.length} onClick={() => void copy()}>
          Copy as Text
        </button>
        <Diagnostics session={session} />
      </div>
      {copied ? (
        <p class="note" role="status">
          {copied}
        </p>
      ) : null}
      {log.error ? (
        <p class="error" role="alert">
          {log.error}
        </p>
      ) : null}
      {shown.length ? (
        <table class="event-list">
          <thead>
            <tr>
              <th scope="col">Time</th>
              <th scope="col">Unit</th>
              <th scope="col">Event</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((r) => (
              <tr key={r.key} class={r.tone ? `tone-${r.tone}` : undefined}>
                <td class="event-time" title={fullTime(r.t)}>
                  {timeText(r.t, now)}
                </td>
                <td class="event-unit">{r.unit || "–"}</td>
                <td class="event-text">
                  <span class="event-kind">{r.kind}</span> {r.text}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <p class="note">No events yet.</p>
      )}
    </section>
  );
}

/** The client's own file in the bundle: what a supporter asks of the browser. */
export function browserFacts(nav: { userAgent?: string; language?: string } | null, jspi: boolean): string {
  return [
    `User agent: ${nav?.userAgent ?? "unknown"}`,
    `Language: ${nav?.language ?? "unknown"}`,
    `JavaScript Promise Integration: ${jspi ? "yes" : "no"}`,
  ].join("\n") + "\n";
}

/** Standard base64 to bytes. */
export function fromBase64(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Save Diagnostics…: the core's zip as a download. */
function Diagnostics(props: { session: Session }) {
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState<{ text: string; bad: boolean } | null>(null);
  const link = useRef<string | null>(null);
  useEffect(
    () => () => {
      if (link.current) URL.revokeObjectURL(link.current);
    },
    [],
  );
  const save = async () => {
    setBusy(true);
    try {
      const jspi = typeof (WebAssembly as unknown as { Suspending?: unknown }).Suspending === "function";
      const d = await props.session.core.call("diagnostics", {
        app: "uConsole for DSPi (web)",
        files: [{ name: "browser.txt", text: browserFacts(typeof navigator === "undefined" ? null : navigator, jspi) }],
      });
      if (link.current) URL.revokeObjectURL(link.current);
      link.current = URL.createObjectURL(new Blob([fromBase64(d.base64) as BlobPart], { type: "application/zip" }));
      const a = document.createElement("a");
      a.href = link.current;
      a.download = d.name;
      a.style.display = "none";
      document.body.appendChild(a);
      a.click();
      a.remove();
      setSaid({ text: `Diagnostics saved as ${d.name}. Nothing was sent anywhere; attach the file to a report if you want to share it.`, bad: false });
    } catch (e) {
      setSaid({ text: `Diagnostics not saved: ${e instanceof Error ? e.message : String(e)}`, bad: true });
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <button type="button" class="save-diagnostics" disabled={busy} onClick={() => void save()}>
        Save Diagnostics…
      </button>
      {said ? (
        <p class={said.bad ? "error diagnostics-said" : "note diagnostics-said"} role={said.bad ? "alert" : "status"}>
          {said.text}
        </p>
      ) : null}
    </>
  );
}
