// WP PGWEB: the device-info screen (PLAN-PG sections 1.1, 1.3, 1.9): the board, the protocol
// generation, the paged protocol's version and pages, the extension envelope, from the core's
// `deviceProtocol` read once per connection (Session.state.protocol). On a paged device, raw
// requests by command id (`rawGetId`/`rawSetId`), each checked whole before it is sent and the
// core's refusals said in words. And the update check (section 1.8): the web has no firmware
// write, so a chosen image is judged with `firmwareTarget` + `firmwareUpdateCheck` and the
// verdict shown (no BOOTSEL, lineage, generation change) for the app that writes it.
//
// The alink sender (`deviceKind` "alinkSender") gets `SenderScreen`: the device info, its audio
// link block and the update check, and no DSP screen.

import { useEffect, useRef, useState } from "preact/hooks";
import type { AlinkInfo, AlinkStatus, DeviceProtocolRecord, FirmwareTargetRecord, FirmwareUpdateVerdictRecord } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import {
  bytesHex,
  envelopeWords,
  generationWords,
  hex,
  kindWords,
  pagesWords,
  parseRaw,
  rawRefusalWords,
  ruleWords,
  verdictWords,
  type RawForm,
} from "./model";
// -- WP WEB7 -- Qt's look: each part a group with its icon and DemiBold title outside the panel,
// the facts as Qt's settings rows (the name at the left, the value at the right edge)
import { Group, Lucide } from "../../ui/Group";
import "./device.css";

const CHIP = (
  <>
    <rect width="16" height="16" x="4" y="4" rx="2" />
    <rect width="6" height="6" x="9" y="9" rx="1" />
    <path d="M15 2v2M15 20v2M2 15h2M2 9h2M20 15h2M20 9h2M9 2v2M9 20v2" />
  </>
);
const TERMINAL = (
  <>
    <path d="m4 17 6-6-6-6" />
    <path d="M12 19h8" />
  </>
);

export function DeviceScreen(props: ScreenProps) {
  const st = useSessionState(props.session);
  return (
    <div class="device-screen">
      <DeviceInfo session={props.session} protocol={st.protocol ?? null} />
      {st.protocol?.generation === "paged" ? <RawById session={props.session} /> : null}
      <UpdateCheck session={props.session} />
    </div>
  );
}

/** The alink sender's page: what it is, its audio link, the update check. */
export function SenderScreen(props: { session: Session }) {
  const st = useSessionState(props.session);
  return (
    <div class="device-screen sender-screen" aria-label="Audio link sender">
      <p class="note sender-note" role="note">
        This is the audio link sender. It has no DSP: no channels, EQ, presets or meters, so those screens are not shown.
      </p>
      <DeviceInfo session={props.session} protocol={st.protocol ?? null} />
      <SenderLink session={props.session} />
      <UpdateCheck session={props.session} />
    </div>
  );
}

function DeviceInfo(props: { session: Session; protocol: DeviceProtocolRecord | null }) {
  const st = useSessionState(props.session);
  const p = props.protocol;
  const dev = st.snapshot?.device;
  const row = (label: string, value: string, cls: string) => (
    <div class={`device-fact ${cls}`}>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
  // -- WP WEB7 -- Qt's "Connected Device" (Settings › About): raw requests are for the paged protocol only, said under it
  return (
    <Group
      title="Connected Device"
      icon={<Lucide paths={CHIP} />}
      class="device-info"
      footer={p && p.generation !== "paged" ? <span class="raw-none">Raw requests by command id need firmware 1.1.7 or later (the paged protocol).</span> : undefined}
    >
      {p ? (
        <dl class="device-facts">
          {row("Board", p.boardName, "device-board")}
          {row("Kind", kindWords(p), "device-kind")}
          {dev ? row("Firmware", `${dev.firmware} (${dev.platform})`, "device-firmware") : null}
          {row("Protocol", generationWords(p), "device-generation")}
          {row("Pages", pagesWords(p), "device-pages")}
          {row("Extensions", envelopeWords(p), "device-envelope")}
        </dl>
      ) : (
        <p class="note">This connection does not say its protocol.</p>
      )}
    </Group>
  );
}

function SenderLink(props: { session: Session }) {
  const [info, setInfo] = useState<AlinkInfo | null>(null);
  const [status, setStatus] = useState<AlinkStatus | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let alive = true;
    const core = props.session.core;
    void (async () => {
      try {
        const i = await core.call("fetchAlinkInfo", undefined);
        const s = await core.call("alinkStatus", undefined);
        if (alive) {
          setInfo(i);
          setStatus(s);
        }
      } catch (e) {
        if (alive) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      alive = false;
    };
  }, [props.session]);
  return (
    <Group title="Audio Link" icon="link" class="sender-link">
      {error ? (
        <p class="error" role="alert">
          {error}
        </p>
      ) : null}
      {info && status ? (
        <dl class="device-facts">
          <div class="device-fact alink-role">
            <dt>Role</dt>
            <dd>{info.role}</dd>
          </div>
          <div class="device-fact alink-state">
            <dt>State</dt>
            <dd>{status.state}</dd>
          </div>
          <div class="device-fact alink-channels">
            <dt>Channels</dt>
            <dd>
              {status.channels} of {info.maxChannels}
            </dd>
          </div>
          <div class="device-fact alink-errors">
            <dt>Errors</dt>
            <dd>
              CRC {status.crc8Errors + status.crc16Errors}, slips {status.bitSlips}, losses {status.lossCount}
            </dd>
          </div>
        </dl>
      ) : error ? null : (
        <p class="note">Reading the audio link…</p>
      )}
    </Group>
  );
}

function RawById(props: { session: Session }) {
  const [form, setForm] = useState<RawForm>({ dir: "get", id: "0x0550", value: "", len: "4", data: "" });
  const [said, setSaid] = useState<{ text: string; bad: boolean } | null>(null);
  const [busy, setBusy] = useState(false);
  const ro = props.session.readOnlyReason;
  const set = (k: keyof RawForm) => (e: Event) => setForm({ ...form, [k]: (e.target as HTMLInputElement | HTMLSelectElement).value });
  const send = async () => {
    const parsed = parseRaw(form);
    if (!parsed.ok) {
      setSaid({ text: `Not sent. ${parsed.why}.`, bad: true });
      return;
    }
    const r = parsed.req;
    if (r.dir === "set" && ro) {
      setSaid({ text: `Not sent: ${ro}`, bad: true });
      return;
    }
    setBusy(true);
    setSaid(null);
    try {
      if (r.dir === "get") {
        const { data } = await props.session.core.call("rawGetId", { id: r.id, value: r.value, len: r.len });
        setSaid({ text: `${hex(r.id, 4)}: ${data.length} byte${data.length === 1 ? "" : "s"}: ${bytesHex(data)}`, bad: false });
      } else {
        await props.session.core.call("rawSetId", { id: r.id, value: r.value, data: r.data });
        setSaid({ text: `${hex(r.id, 4)}: the device took it.`, bad: false });
        void props.session.refresh();
      }
    } catch (e) {
      setSaid({ text: rawRefusalWords(r.id, e), bad: true });
    } finally {
      setBusy(false);
    }
  };
  return (
    <Group
      title="Raw Request by Id"
      icon={<Lucide paths={TERMINAL} />}
      class="raw-id"
      footer="A paged command by its id, e.g. 0x0550. What the console guards is refused with nothing sent: flash, restart and bulk commands, confirmed operations, the core voltage."
    >
      <form
        class="raw-form"
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
      >
        <label>
          Direction
          <select aria-label="Direction" value={form.dir} onChange={set("dir")}>
            <option value="get">GET (read)</option>
            <option value="set">SET (write)</option>
          </select>
        </label>
        <label>
          Command id
          <input aria-label="Command id" value={form.id} onInput={set("id")} spellcheck={false} />
        </label>
        <label>
          wValue
          <input aria-label="wValue" value={form.value} onInput={set("value")} placeholder="0" spellcheck={false} />
        </label>
        {form.dir === "get" ? (
          <label>
            Length
            <input aria-label="Length" value={form.len} onInput={set("len")} spellcheck={false} />
          </label>
        ) : (
          <label class="raw-data">
            Data
            <input aria-label="Data" value={form.data} onInput={set("data")} placeholder="bytes, e.g. 0x00 0x80" spellcheck={false} />
          </label>
        )}
        <button type="submit" disabled={busy || (form.dir === "set" && !!ro)}>
          Send
        </button>
      </form>
      {said ? (
        <p class={said.bad ? "error raw-said" : "note raw-said"} role="status" aria-label="Raw result">
          {said.text}
        </p>
      ) : null}
    </Group>
  );
}

function UpdateCheck(props: { session: Session }) {
  const firmware = useSessionState(props.session).snapshot?.device.firmware ?? ""; // -- WP WEB7 --
  const file = useRef<HTMLInputElement>(null);
  const [target, setTarget] = useState<FirmwareTargetRecord | null>(null);
  const [result, setResult] = useState<{ name: string; v: FirmwareUpdateVerdictRecord } | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const choose = async (input: HTMLInputElement) => {
    const f = input.files?.[0];
    input.value = "";
    if (!f) return;
    setBusy(true);
    setError("");
    setResult(null);
    try {
      // The target is read once on this connection (before any bootloader entry) and kept.
      const t = target ?? (await props.session.core.call("firmwareTarget", undefined));
      setTarget(t);
      const image = Array.from(new Uint8Array(await f.arrayBuffer()));
      const v = await props.session.core.call("firmwareUpdateCheck", { image, target: t });
      setResult({ name: f.name, v });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const v = result?.v;
  const rule = v ? ruleWords(v) : "";
  return (
    <Group
      title="Firmware Update Check"
      icon={<Lucide paths={CHIP} />}
      class="update-check"
      footer={
        <>
          This web console does not write firmware. Choose an image to see whether the app that writes it (the desktop console, <code>uconsole flash</code>, a hub rollout) would update
          this board with it, and what changes.
        </>
      }
    >
      {/* -- WP WEB7 -- Qt's Firmware page: the version on the device beside the image's */}
      {firmware ? (
        <dl class="device-facts">
          <div class="device-fact device-version">
            <dt>Connected device</dt>
            <dd>{firmware}</dd>
          </div>
        </dl>
      ) : null}
      <div class="update-check-pick">
        <button type="button" disabled={busy} onClick={() => file.current?.click()}>
          Check an Image…
        </button>
        <input ref={file} type="file" hidden accept=".uf2" aria-label="Firmware image to check" onChange={(e) => void choose(e.target as HTMLInputElement)} />
      </div>
      {error ? (
        <p class="error" role="alert">
          {error}
        </p>
      ) : null}
      {v && result ? (
        <div class={`update-verdict ${v.verdict.allowed ? "ok" : "bad"}`} role="status" aria-label="Update verdict" data-rule={v.rule} data-decision={v.verdict.decision}>
          <p>
            <strong class="update-verdict-head">{verdictWords(v)}</strong> for {result.name}.
          </p>
          {rule ? <p class="update-rule">{rule}</p> : null}
          <p class="update-message">{v.verdict.message}</p>
          {v.verdict.confirmationPhrase && v.verdict.decision === "needsConfirmation" ? (
            <p class="update-phrase">
              The phrase to type there: <code>{v.verdict.confirmationPhrase}</code>
            </p>
          ) : null}
          {v.generationChange ? <p class="update-generation">Generation change: {v.generationChange}</p> : null}
        </div>
      ) : null}
    </Group>
  );
}
