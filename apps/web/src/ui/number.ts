// -- WP WEB -- A typed number as the desktop app's fields read it (Qt's `BorderedField.number()`
// since USR1): spaces (also no-break and thin ones) dropped, the minus sign U+2212 and the other
// dashes as "-", one decimal comma as the point, and a unit typed after the number dropped. Every
// number field of the console reads its text through `plainNumber` before its own rules (a
// frequency's "k", a note name, a field's range).

const SPACES = /[\s   ]+/g;
const DASHES = /[−‒–﹣－]/g;

/**
 * `text` with the number's own spelling made plain ("−2,5 dB" with unit "dB" is "-2.5"), or ""
 * when it cannot be one: a comma beside a point, or two commas. Units match without case, the
 * longest first, so "dbfs" goes before "db".
 */
export function plainNumber(text: string, units: readonly string[] = []): string {
  let s = text.replace(SPACES, "").replace(DASHES, "-");
  const lower = s.toLowerCase();
  const unit = [...units]
    .map((u) => u.replace(SPACES, "").toLowerCase())
    .filter((u) => u && lower.endsWith(u))
    .sort((a, b) => b.length - a.length)[0];
  if (unit) s = s.slice(0, s.length - unit.length);
  if (s.includes(",")) {
    if (s.includes(".") || s.indexOf(",") !== s.lastIndexOf(",")) return "";
    s = s.replace(",", ".");
  }
  return s;
}

/** A typed number: "3,5", "−2.5", "+1.5 dB" (with "dB" among `units`), ".5"; null when it is not one. */
export function readNumber(text: string, units: readonly string[] = []): number | null {
  const s = plainNumber(text, units);
  if (!/^[-+]?(\d+\.?\d*|\.\d+)$/.test(s)) return null;
  const v = Number(s);
  return Number.isFinite(v) ? v : null;
}
