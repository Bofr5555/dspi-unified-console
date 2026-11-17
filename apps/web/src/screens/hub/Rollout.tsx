// WP P3WRO: the firmware rollout on the hub screen (PLAN-P3 F6, F8; hub-api.md section 21),
// shown only to an Admin of a hub with the `rollout` cap. The image store with each image's
// version, platform, core-voltage stamp and "verified" or "not verified"; an image picked
// from a file and uploaded (8 MiB refused before a byte goes); a rollout started on a DSPi
// or a group from a dialog that lists the steps and says what the DSPi will do, with "Mute
// First" or the while-playing choice spelled out; each step live from
// `fw.rollout.changed`; the result's lines and "Recall the Export" when the settings
// changed; Cancel (taken before the bootloader entry, refused after it, and said so); the
// `rollout_failed` alarm with its step. Over a plaintext connection the hub refuses all of
// it: the TLS notice says so.

import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type { HubFleetTarget, HubRolloutDeviceRecord, HubRolloutRecord } from "../../core/protocol";
import type { Ask } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { noticeRow } from "../../ui/texts";
import type { HubFeed, HubView } from "./feed";
import { shortTime } from "./model";
import {
  currentDevice,
  failedAlarms,
  fileProblem,
  imageRows,
  mayCancel,
  recallOffer,
  resultLines,
  RolloutFeed,
  type RolloutView,
  type Said,
  sizeText,
  stateWord,
  stepRows,
  STEPS,
  targetName,
  targetOf,
} from "./rollout-model";

/** The rollouts listed, newest first. */
const SHOWN_ROLLOUTS = 5;

function useRolloutView(feed: RolloutFeed): RolloutView {
  const [, bump] = useState(0);
  useEffect(() => feed.subscribe(() => bump((n) => n + 1)), [feed]);
  return feed.view;
}

function stepTitle(step: string): string {
  return noticeRow(`rollout.${step}`)?.title ?? step;
}

export function Rollout(props: { session: Session; hub: HubFeed; view: HubView; ask: Ask; readOnly: string }) {
  const { hub, view: hubView, session } = props;
  const feed = useMemo(() => new RolloutFeed(session), [session]);
  useEffect(() => {
    feed.start();
    return () => feed.stop();
  }, [feed]);
  const view = useRolloutView(feed);
  const [said, setSaid] = useState<Said | null>(null);
  const [busy, setBusy] = useState(false);
  const [target, setTarget] = useState("");
  const [image, setImage] = useState("");
  const file = useRef<HTMLInputElement>(null);
  const images = imageRows(view.images ?? []);
  const online = hubView.devices.filter((d) => d.online);
  const running = view.rollouts.find((r) => r.state === "running") ?? null;
  const names = { device: (s: string) => hub.deviceName(s), groups: view.groups };
  const alarms = failedAlarms(hubView.alarms, view.rollouts);
  const closed = !!view.tls || !!props.readOnly;
  const chosenTarget = target || (online[0] ? `serial:${online[0].serial}` : "");
  const chosenImage = image || images.at(-1)?.name || "";

  const run = async (f: () => Promise<Said | null>) => {
    setBusy(true);
    setSaid(null);
    try {
      const r = await f();
      if (r) setSaid(r);
    } finally {
      setBusy(false);
    }
  };

  const choose = async (input: HTMLInputElement) => {
    const f = input.files?.[0];
    input.value = "";
    if (!f) return;
    await run(async () => {
      // 8 MiB and the name are checked before the file is even read: nothing is sent then.
      const problem = fileProblem(f.name, f.size);
      if (problem) return { text: problem, bad: true, notice: null };
      const r = await feed.upload(f.name, new Uint8Array(await f.arrayBuffer()));
      if (!r.bad) setImage(f.name);
      return r;
    });
  };

  const start = async () => {
    const t = targetOf(chosenTarget);
    if (!t || !chosenImage) return;
    const group = "group" in t && !!t.group;
    const who = targetName(group ? `group:${t.group}` : chosenTarget.slice(7), names);
    const muted = hubView.caps?.groupParams ? await feed.muted(t) : null;
    const a = await props.ask({
      title: "Update the Firmware",
      body: <StartPlan who={who} group={group} image={chosenImage} muted={muted} />,
      buttons:
        muted === true
          ? [
              { label: "Start", value: "start" },
              { label: "Cancel", value: "cancel" },
            ]
          : [
              ...(hubView.caps?.groupParams ? [{ label: "Mute First", value: "mute" }] : []),
              { label: "Start While Playing", value: "live", danger: true },
              { label: "Cancel", value: "cancel" },
            ],
    });
    if (a === "cancel") return;
    await run(async () => {
      if (a === "mute") {
        const m = await feed.muteFirst(t);
        if (m.bad) return m;
      }
      return go(t, a === "live");
    });
  };

  const go = async (t: HubFleetTarget, whileLive: boolean): Promise<Said> => {
    const r = await feed.startRollout(t, chosenImage, whileLive);
    if (r.code === "confirmation_required")
      return { ...r, text: `${r.text} The DSPi is not muted: choose Mute First, or Start While Playing.` };
    return r;
  };

  return (
    <section class="card hub-rollout" aria-label="Firmware rollout">
      <div class="card-head">
        <h2>Firmware rollout</h2>
      </div>
      {view.tls ? (
        <div class="hub-rollout-tls error" role="alert" aria-label={view.tls.title} data-key={view.tls.key}>
          <strong>{view.tls.title}.</strong> {view.tls.reason} {view.tls.action}
        </div>
      ) : null}
      {alarms.map(({ alarm, step }) => (
        <div key={alarm.id} class="hub-rollout-alarm bad" role="alert" aria-label="Firmware update failed" data-key={alarm.noticeKey || "alarm.rollout_failed"} data-step={step}>
          <strong>{noticeRow("alarm.rollout_failed")?.title ?? "Firmware update failed"}</strong> on {hub.deviceName(alarm.serial)}
          {step ? `, at the step “${stepTitle(step)}”` : ""}. <span class="hub-rollout-detail">{alarm.message}</span>
        </div>
      ))}
      {view.error ? (
        <p class="error" role="alert">
          {view.error}
        </p>
      ) : null}
      {said ? (
        <p class={said.bad ? "error" : "note"} role="status" aria-label="Firmware action" data-key={said.notice?.key}>
          {said.text}
          {said.bad && said.notice?.action ? ` ${said.notice.action}` : ""}
        </p>
      ) : null}

      <h3>Images on the hub</h3>
      {images.length ? (
        <div class="hub-table-wrap">
          <table class="hub-table hub-images">
            <thead>
              <tr>
                <th scope="col">Image</th>
                <th scope="col">Version</th>
                <th scope="col">Platform</th>
                <th scope="col">Core voltage</th>
                <th scope="col">Check</th>
                <th scope="col">Size</th>
                <th scope="col">Added</th>
              </tr>
            </thead>
            <tbody>
              {images.map((i) => (
                <tr key={i.name} aria-label={i.name} class="hub-image-row">
                  <td class="hub-image-name">{i.name}</td>
                  <td class="hub-image-version">{i.version}</td>
                  <td>{i.platform}</td>
                  <td class="hub-image-stamp">
                    {i.stamp}
                    {i.k340 ? <span class="hub-image-k340"> (K340)</span> : null}
                  </td>
                  <td class={i.verified ? "hub-image-verified ok" : "hub-image-verified warn"}>{i.label}</td>
                  <td>{i.size}</td>
                  <td>{i.added}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : view.images && !view.tls ? (
        <p class="note">No images on the hub yet.</p>
      ) : null}
      <div class="hub-rollout-upload">
        <button type="button" disabled={busy || closed || !!view.uploading} onClick={() => file.current?.click()}>
          Add Image…
        </button>
        <input ref={file} type="file" hidden accept=".uf2" aria-label="Firmware image to upload" onChange={(e) => void choose(e.target as HTMLInputElement)} />
        {view.uploading ? (
          <span class="hub-rollout-progress">
            <progress aria-label="Upload" /> Uploading {view.uploading.name} ({sizeText(view.uploading.size)})…
          </span>
        ) : (
          <span class="note">Images up to 8 MiB. An image not in the release's list shows as “not verified”.</span>
        )}
      </div>

      <h3>Update</h3>
      <div class="hub-rollout-start">
        <label>
          DSPi or group
          <select aria-label="DSPi or group" value={chosenTarget} disabled={closed} onChange={(e) => setTarget((e.target as HTMLSelectElement).value)}>
            {online.map((d) => (
              <option key={d.serial} value={`serial:${d.serial}`}>
                {d.name || d.serial} ({d.serial})
              </option>
            ))}
            {view.groups.map((g) => (
              <option key={g.id} value={`group:${g.id}`}>
                Group {g.name} ({g.members.length})
              </option>
            ))}
          </select>
        </label>
        <label>
          Image
          <select aria-label="Image" value={chosenImage} disabled={closed || !images.length} onChange={(e) => setImage((e.target as HTMLSelectElement).value)}>
            {images.map((i) => (
              <option key={i.name} value={i.name}>
                {i.name} ({i.version}, {i.label})
              </option>
            ))}
          </select>
        </label>
        <button type="button" disabled={busy || closed || !!running || !chosenTarget || !chosenImage} onClick={() => void start()}>
          Start Update…
        </button>
      </div>
      {running ? <p class="note">One rollout runs at a time on a hub.</p> : null}

      {view.rollouts.slice(0, SHOWN_ROLLOUTS).map((r) => (
        <RolloutRecord key={r.id} r={r} feed={feed} names={names} busy={busy} readOnly={!!props.readOnly} run={run} />
      ))}
    </section>
  );
}

/** The start dialog's body: the steps in words, what the DSPi will do, and the mute. */
function StartPlan(props: { who: string; group: boolean; image: string; muted: boolean | null }) {
  return (
    <div class="hub-rollout-plan">
      <p>
        Update {props.who} to <strong>{props.image}</strong>
        {props.group ? ", one DSPi after another, stopping at the first failure" : ""}. The hub:
      </p>
      <ol class="hub-rollout-steps-plan">
        {STEPS.map((s) => (
          <li key={s}>
            <strong>{stepTitle(s)}.</strong> {noticeRow(`rollout.${s}`)?.reason ?? ""}
          </li>
        ))}
      </ol>
      <p>
        {props.group ? "Each DSPi" : "The DSPi"} is offline for about 30 s: it restarts into its bootloader, takes the new firmware and restarts. Its settings are
        saved on the hub first. Once it is in its bootloader the update cannot be cancelled.
      </p>
      {props.muted === true ? null : (
        <p class="warn" data-key="rollout.check">
          {props.muted === false ? "This DSPi is not muted." : "The hub could not say whether every DSPi is muted."} The hub updates only a muted DSPi unless you accept that it
          plays: <strong>Mute First</strong> mutes it now, then starts. <strong>Start While Playing</strong> updates it as it is: its sound stops mid-programme for about
          30 s and comes back after the restart at the volume it had.
        </p>
      )}
    </div>
  );
}

function RolloutRecord(props: {
  r: HubRolloutRecord;
  feed: RolloutFeed;
  names: { device: (s: string) => string; groups: RolloutView["groups"] };
  busy: boolean;
  readOnly: boolean;
  run: (f: () => Promise<Said | null>) => Promise<void>;
}) {
  const { r, feed } = props;
  const cancellable = mayCancel(r);
  const now = currentDevice(r);
  return (
    <div class={`hub-rollout-record ${r.state}`} role="group" aria-label={`Rollout ${r.id}`} data-state={r.state}>
      <div class="hub-rollout-head">
        <strong>
          {r.id}: {targetName(r.target, props.names)}
        </strong>
        <span class="hub-rollout-state">{stateWord(r.state)}</span>
      </div>
      <p class="note">
        {r.image}
        {r.imageVersion ? ` (${r.imageVersion})` : ""}, by {r.by || "–"}, {shortTime(r.startedAt)}
        {r.whileLive ? ", while playing" : ""}
        {r.endedAt ? `; ended ${shortTime(r.endedAt)}` : ""}
      </p>
      {r.state === "running" ? (
        <div class="hub-rollout-cancel">
          {/* Enabled in the client read-only mode too: the core lets fw.rollout.cancel out there, as BulkApply's Revert Now. */}
          <button type="button" disabled={props.busy} onClick={() => void props.run(() => feed.cancel(r.id))}>
            Cancel Rollout
          </button>
          {cancellable ? null : (
            <span class="note">
              {now ? `${props.names.device(now.serial)} is past its bootloader entry` : "Past the bootloader entry"}: a cancel is refused until this DSPi is done.
            </span>
          )}
        </div>
      ) : null}
      {r.devices.map((d) => (
        <DeviceEntry key={d.serial} d={d} name={props.names.device(d.serial)} feed={feed} busy={props.busy} readOnly={props.readOnly} run={props.run} />
      ))}
    </div>
  );
}

function DeviceEntry(props: {
  d: HubRolloutDeviceRecord;
  name: string;
  feed: RolloutFeed;
  busy: boolean;
  readOnly: boolean;
  run: (f: () => Promise<Said | null>) => Promise<void>;
}) {
  const { d } = props;
  const lines = resultLines(d);
  const offer = recallOffer(d);
  return (
    <div class={`hub-rollout-device ${d.state}`} aria-label={`${props.name} (${d.serial})`} data-state={d.state}>
      <div class="hub-rollout-head">
        <span>
          {props.name} <span class="hub-serial">{d.serial}</span>
        </span>
        <span class="hub-rollout-state">{stateWord(d.state)}</span>
      </div>
      {d.state === "not_tried" ? null : (
        <ol class="hub-rollout-steps">
          {stepRows(d).map((s) => (
            <li key={s.step} class={`hub-step ${s.state}`} data-step={s.step} data-state={s.state} aria-label={stepTitle(s.step)}>
              <span class="hub-step-mark" aria-hidden="true">
                {s.state === "done" ? "✓" : s.state === "failed" ? "✗" : s.state === "running" ? "…" : "·"}
              </span>
              <span class="hub-step-title">{stepTitle(s.step)}</span>
              {s.text ? <span class="hub-step-text"> {s.text}</span> : null}
            </li>
          ))}
        </ol>
      )}
      {d.failed ? (
        <p class="error" role="alert" aria-label="Where it stopped" data-key={d.failed.noticeKey || `rollout.${d.failed.step}`}>
          Stopped at “{stepTitle(d.failed.step)}”: {d.failed.text} {noticeRow(`rollout.${d.failed.step}`)?.action ?? ""}
        </p>
      ) : null}
      {lines.length ? (
        <ul class="hub-rollout-result" aria-label="Result">
          {lines.map((l) => (
            <li key={l}>{l}</li>
          ))}
        </ul>
      ) : null}
      {offer ? (
        <button type="button" disabled={props.busy || props.readOnly} onClick={() => void props.run(() => props.feed.recall(offer.serial, offer.id))}>
          Recall the Export
        </button>
      ) : null}
    </div>
  );
}
