// WP SW3, WB6e: the scene bar. The installation's scenes as numbered recall
// buttons; a recall is `system.scene.apply`, which acts at one alink frame on
// every unit, and the bar then shows what each unit said: queued, applied,
// late or refused. A scene the role may not run is refused whole, and the
// hub's answer names the op. Recall asks first when this session's device has
// unsaved changes, as WB6e's slice says.

import type { Ask } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { WritesByElement } from "./AlinkClock";
import type { SystemFeed } from "./feed";
import { useFeed } from "./SystemScreen";
import { mayWrite, opWord, subjectName, writeNote } from "./model";

export function SceneBar(props: { feed: SystemFeed; session: Session; ask: Ask }) {
  const st = useFeed(props.feed);
  if (st.kind !== "hub" || !st.model) return null;
  const scenes = st.model.scenes;
  const allowed = mayWrite(st.role, "scene") && !props.feed.readOnly;
  const run = st.scene;
  const recall = async (id: string, name: string) => {
    if (props.session.dirty) {
      const a = await props.ask({
        title: "Recall Scene",
        body: (
          <p>
            This DSPi has unsaved changes. Recalling “{name}” may change it; the changes stay unsaved.
          </p>
        ),
        buttons: [
          { label: "Recall", value: "recall" },
          { label: "Cancel", value: "cancel" },
        ],
      });
      if (a !== "recall") return;
    }
    await props.feed.recall(id);
  };
  const names = (id: string) => subjectName(st.model, id);
  const opOf = (element: string, what: string, reason?: string) => {
    // -- fixup lk1 -- through the sender the one write carries the whole scene: "Scene Closed", never a refused "Mute"
    // -- WP HWLK1F -- but a unit's own result (applied now, not connected) keeps its op
    if (run?.report?.plan.path === "superframe" && !reason) return `Scene ${run.name}`;
    const plan = run?.report?.plan;
    const op = [...(plan?.ops ?? []), ...(plan?.direct ?? []), ...(plan?.unscheduled ?? [])].find((o) => o.element === element && o.op === what);
    return op ? opWord(op.op, op.arg, op.value) : what;
  };
  return (
    <div class="sys-scenes" aria-label="Scenes">
      <div class="sys-scene-row">
        <span class="sys-label">Scenes</span>
        {scenes.length ? null : <span class="sys-muted">The installation has no scenes</span>}
        {scenes.map((s, i) => (
          <button
            type="button"
            key={s.id}
            class={`sys-scene${st.recalled?.scene === s.id ? " active" : ""}`}
            aria-current={st.recalled?.scene === s.id ? "true" : undefined}
            aria-label={`Recall ${s.name || s.id}`}
            disabled={!allowed || run?.running}
            title={allowed ? undefined : writeNote(st.role, "scene")}
            onClick={() => void recall(s.id, s.name || s.id)}
          >
            <span class="sys-scene-n">{i + 1}</span> {s.name || s.id}
          </button>
        ))}
      </div>
      {run ? (
        <div class="sys-scene-result" aria-label="Scene result" aria-live="polite">
          {run.running ? <span class="sys-muted">Recalling {run.name}…</span> : null}
          {run.error ? (
            <p class="error" role="alert">
              {run.name}: refused. {run.error}
            </p>
          ) : null}
          {/* -- fixup sw3x -- a recall the hub took none of: said as such, never marked active */}
          {run.refusal ? (
            <p class="error" role="alert">
              {run.refusal}
            </p>
          ) : null}
          {run.report ? (
            <>
              {/* -- WP HWLK1F -- the core's sentence, the same every client shows; the frame and lead beside it */}
              {run.refusal ? null : <p class="sys-note">
                <strong>{run.outcome}</strong>
                {run.report.plan.frame ? ` (${run.report.plan.leadFrames} frames ahead${run.report.onTime ? ", on time on every unit" : ""})` : ""}
              </p>}
              <WritesByElement
                writes={run.report.writes}
                names={names}
                label="Per unit"
                what={(w) => opOf(w.element, w.what, w.reason)}
              />
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
