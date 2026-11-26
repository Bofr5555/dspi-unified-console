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
// -- WP P2WB1 -- "On the hub": the hub's shared history of this DSPi, with who did it
import { useHubView } from "../hub/HubScreen";
import { HubHistory } from "./HubHistory";
import { EmptyState } from "../../ui/design"; // -- WP WEB --

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
  // -- WP P2WB1 -- which log: this console's, or the hub's (when the hub keeps one)
  const hubView = useHubView(props.hub);
  const hubLog = !!hubView?.caps?.events;
  const [source, setSource] = useState<"console" | "hub">("console");
  const onHub = hubLog && source === "hub";

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

  // -- fixup wb8 -- the Clipboard API exists only on a secure page; a hub's page over plain
  // http has none: then a selected text and the older copy command, and failing that the
  // text shown selected, to copy by hand.
  const [manual, setManual] = useState<string | null>(null);
  const copy = async () => {
    const text = rowsText(shown);
    const done = shown.length === 1 ? "1 event copied." : `${shown.length} events copied.`;
    setManual(null);
    try {
      if (!navigator.clipboard) throw new Error("no clipboard");
      await navigator.clipboard.writeText(text);
      setCopied(done);
      return;
    } catch {
      // The older way below.
    }
    if (copyBySelection(text)) {
      setCopied(done);
      return;
    }
    setCopied("The browser did not copy here: the text is selected below; copy it with the keyboard or the menu.");
    setManual(text);
  };

  return (
    // -- WP WEB2 -- (§8, §9) the log is the page, not a card around it: its name a section label, the tools at its right
    <section class="events" aria-label="Event log">
      <div class="events-head">
        <h2 class="section-label">Event log</h2>
        {/* -- WP P2WB1 -- */}
        {hubLog ? (
          <label class="events-source">
            Source
            <select value={source} onChange={(e) => setSource((e.target as HTMLSelectElement).value === "hub" ? "hub" : "console")}>
              <option value="console">This console</option>
              <option value="hub">On the hub</option>
            </select>
          </label>
        ) : null}
        {/* -- WP P2WB1 -- the unit and the copy are this console's log's */}
        {onHub ? null : <label class="events-unit">
          Unit
          <select value={unit} onChange={(e) => setUnit((e.target as HTMLSelectElement).value)}>
            <option value="">Every unit</option>
            {units.map((u) => (
              <option key={u.id} value={u.id}>
                {u.name}
              </option>
            ))}
          </select>
        </label>}
        {onHub ? null : <button type="button" disabled={!shown.length} onClick={() => void copy()}>
          Copy as Text
        </button>}
        <Diagnostics session={session} />
      </div>
      {copied ? (
        <p class="note" role="status">
          {copied}
        </p>
      ) : null}
      {manual !== null ? (
        <textarea
          class="events-manual"
          aria-label="Events as text"
          readOnly
          value={manual}
          ref={(el) => {
            if (el && document.activeElement !== el) {
              el.focus();
              el.select();
            }
          }}
        />
      ) : null}
      {log.error ? (
        <p class="error" role="alert">
          {log.error}
        </p>
      ) : null}
      {/* -- WP P2WB1 -- the hub's records instead of this console's */}
      {onHub ? (
        <HubHistory session={session} />
      ) : shown.length ? (
        <table class="event-list ds-table">
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
        // -- WP WEB -- DL3b's empty state
        <EmptyState icon="list-ordered" title="No events yet" />
      )}
    </section>
  );
}

/** -- fixup wb8 -- `text` copied through a selected, unseen text area and `execCommand("copy")`; whether it went. */
export function copyBySelection(text: string): boolean {
  if (typeof document === "undefined") return false;
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.setAttribute("readonly", "");
  ta.style.position = "fixed";
  ta.style.top = "0";
  ta.style.left = "0";
  ta.style.width = "1px";
  ta.style.height = "1px";
  ta.style.opacity = "0";
  const back = document.activeElement as HTMLElement | null;
  document.body.appendChild(ta);
  let ok = false;
  try {
    ta.select();
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  } finally {
    ta.remove();
    back?.focus?.();
  }
  return ok;
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
