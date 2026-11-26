// WP UB7 (UXP B7, C3): the speaker layout in one line where it is used, not edited (Qt's
// SpeakersSummary): Measure and Auto-tune ("L: woofer Out 1, tweeter Out 2 · R: … · confirmed"),
// with Edit in Speakers… to the one table. What each run measures stays the tab's own tick boxes.

import { baseOf, layoutSummary, type LayoutView } from "./model";
import "./speakers.css";

export function LayoutSummary(props: { view: LayoutView; loaded: boolean; onEdit?: () => void }) {
  const { view, loaded, onEdit } = props;
  const text = loaded ? layoutSummary(baseOf(view), !!view.stored) : "";
  if (!text) return null;
  return (
    <div class="layout-summary">
      <span class="layout-summary-text" role="status" aria-label="Speaker layout summary">
        {text}
      </span>
      {onEdit ? (
        <button type="button" onClick={onEdit}>
          Edit in Speakers…
        </button>
      ) : null}
    </div>
  );
}
