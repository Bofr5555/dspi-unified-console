// -- WP WEB4 -- The signal generator's tiles draw their waveform, as Qt's SiggenTile does: a small
// path for each kind of signal, in a 100 × 28 box, chosen by the tile's words ("Sine", "Log Swp").

const W = 100;
const H = 28;
const MID = H / 2;

/** A path through `n + 1` points of `f` over [0, 1] (f gives −1 … 1). */
function trace(f: (t: number) => number, n = 120): string {
  const pts: string[] = [];
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    const y = MID - Math.max(-1, Math.min(1, f(t))) * (MID - 3);
    pts.push(`${i ? "L" : "M"}${(t * W).toFixed(1)},${y.toFixed(1)}`);
  }
  return pts.join("");
}

/** A fixed pseudo-random sequence (the same picture every time). */
function noise(seed: number): (i: number) => number {
  return (i) => {
    const x = Math.sin((i + 1) * 12.9898 + seed * 78.233) * 43758.5453;
    return (x - Math.floor(x)) * 2 - 1;
  };
}

/** The waveform of the tile named `tile`, as an SVG path in a 100 × 28 box. */
export function wavePath(tile: string): string {
  const k = tile.toLowerCase();
  const pi2 = Math.PI * 2;
  if (k.includes("square")) return `M0,${MID}` + [0, 1, 2].map((c) => `L${c * 33},5L${c * 33 + 16.5},5L${c * 33 + 16.5},23L${(c + 1) * 33},23`).join("") + `L${W},23`;
  if (k.includes("white")) {
    const r = noise(1);
    return trace((t) => r(Math.round(t * 90)) * 0.9, 90);
  }
  if (k.includes("pink")) {
    const r = noise(2);
    return trace((t) => 0.55 * Math.sin(t * pi2 * 2.2) + 0.35 * r(Math.round(t * 70)), 70);
  }
  if (k.includes("log")) return trace((t) => Math.sin(pi2 * 1.2 * (Math.exp(t * 2.3) - 1)));
  if (k.includes("lin")) return trace((t) => Math.sin(pi2 * (1 + 5 * t) * t * 1.6));
  if (k.includes("step")) return `M0,22` + [0, 1, 2, 3].map((s) => `L${s * 25 + 4},${22 - s * 5}L${s * 25 + 20},${22 - s * 5}`).join("") + `L${W},7`;
  if (k.includes("impulse")) return `M0,${MID}L48,${MID}L50,3L52,${MID}L${W},${MID}`;
  if (k.includes("click")) return `M0,${MID}` + [18, 50, 82].map((x) => `L${x - 2},${MID}L${x},4L${x + 2},${MID + 6}L${x + 4},${MID}`).join("") + `L${W},${MID}`;
  if (k.includes("polar")) return `M0,${MID}L30,${MID}L34,4L42,${MID + 4}L50,${MID}L${W},${MID}`;
  if (k.includes("burst")) return trace((t) => (t > 0.25 && t < 0.55 ? Math.sin(t * pi2 * 10) : 0));
  if (k.includes("2-tone") || k.includes("two")) return trace((t) => 0.5 * Math.sin(t * pi2 * 2) + 0.45 * Math.sin(t * pi2 * 9));
  if (k.includes("multi")) return trace((t) => 0.35 * Math.sin(t * pi2 * 2) + 0.3 * Math.sin(t * pi2 * 5.3 + 1) + 0.25 * Math.sin(t * pi2 * 11.7 + 2));
  if (k.includes("isp")) return `M0,22L8,22L14,6L30,4L46,6L52,22L${W},22`;
  if (k.includes("chan")) return trace((t) => (Math.floor(t * 3) % 1 === 0 && (t * 3) % 1 > 0.2 && (t * 3) % 1 < 0.75 ? Math.sin(t * pi2 * 14) * 0.8 : 0));
  return trace((t) => Math.sin(t * pi2 * 2));
}

export const WAVE_BOX = { width: W, height: H } as const;
