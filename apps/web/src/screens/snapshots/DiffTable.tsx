// WP P1WB1: a document diff (P1DF's rows: block, section, item, before, after) as a table that
// wraps at 375 px instead of scrolling the page sideways.

import type { ConfigDiff, ImportBlock } from "../../core/protocol";

/** The blocks in the import dialog's order, with their names. */
export const BLOCKS: { key: ImportBlock; bit: number; label: string }[] = [
  { key: "eq", bit: 1 << 2, label: "EQ" },
  { key: "crossover", bit: 1 << 3, label: "Crossover" },
  { key: "routing", bit: 1 << 4, label: "Routing and matrix" },
  { key: "outputs", bit: 1 << 5, label: "Outputs" },
  { key: "limiters", bit: 1 << 6, label: "Limiters" },
  { key: "effects", bit: 1 << 7, label: "Effects" },
  { key: "volumes", bit: 1 << 0, label: "Volume levels" },
  { key: "hardware", bit: 1 << 1, label: "Hardware I/O" },
  { key: "names", bit: 1 << 8, label: "Names" },
];

export function blockLabel(key: string): string {
  return BLOCKS.find((b) => b.key === key)?.label ?? key;
}

/** At most this many rows are drawn; the count says how many there are. */
const MAX_ROWS = 200;

export function DiffTable(props: { diff: ConfigDiff; label: string }) {
  const d = props.diff;
  if (d.count === 0) return <p class="note">No differences.</p>;
  const rows = d.rows.slice(0, MAX_ROWS);
  return (
    <div class="diff">
      <p class="note">
        {d.count === 1 ? "1 value differs" : `${d.count} values differ`}
        {d.rows.length > MAX_ROWS ? ` (the first ${MAX_ROWS} shown)` : ""}.
      </p>
      <table class="diff-table" aria-label={props.label}>
        <thead>
          <tr>
            <th scope="col">Setting</th>
            <th scope="col">Before</th>
            <th scope="col">After</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i}>
              <td>
                <span class="diff-block">{blockLabel(r.block)}</span> {r.section}
                {r.item ? ` · ${r.item}` : ""}
              </td>
              <td>{r.before}</td>
              <td>{r.after}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {d.notes.length ? (
        <ul class="change-list notes" aria-label="Notes">
          {d.notes.map((n, i) => (
            <li key={i}>{n}</li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
