// -- WP WEB4 -- DL2's step dots for the auto-tune (Layout · Proposal · Applied). The icons and the
// status chip that were here are ui/design.tsx's `Glyph` and `StatusChip` now (INTD).

/** DL2's step dots: the steps in a row, the reached ones in the accent, the current one named. */
export function StepDots(props: { steps: readonly string[]; current: number; label: string }) {
  return (
    <ol class="step-dots" aria-label={props.label}>
      {props.steps.map((s, i) => (
        <li key={s} class={i < props.current ? "done" : i === props.current ? "now" : undefined} aria-current={i === props.current ? "step" : undefined}>
          <span class="step-dot" aria-hidden="true" />
          <span class="step-name">{s}</span>
        </li>
      ))}
    </ol>
  );
}
