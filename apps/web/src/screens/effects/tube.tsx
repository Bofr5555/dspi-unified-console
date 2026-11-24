// -- WP WEB6 -- The Tube page's picture tile (Qt's TubeIllustration, the Mac's TubeGeometry): the tube
// of the chosen type's family, drawn on the 120 × 200 design grid (x -60..60 about the axis, y from
// the top to the pin tips), its heater glowing while the stage is on. The family is the core's
// (`tubeFamily`); the drawing is the page's own, as Qt's is its own.

import { useId } from "preact/hooks";

interface Plate {
  rect: [number, number, number, number];
  fins?: number;
  ribs?: number[];
  mesh?: boolean;
}
interface Geometry {
  /** The envelope's left edge as (half-width, y), bottom to top; mirrored, closed by a dome at `top`. */
  profile: [number, number][];
  top: number;
  metal?: boolean;
  base?: { top: number; bottom: number; topHalf: number; bottomHalf: number; key: boolean };
  pins: { xs: number[]; top: number; bottom: number; width: number };
  getter?: [number, number];
  micas?: [number, number][];
  rods?: [number, number, number][];
  plates?: Plate[];
  filament?: [number, number][];
  glows: [number, number, number][];
  ring?: [number, number];
}

const NOVAL_TRIODE: Geometry = {
  profile: [
    [28, 166],
    [28, 72],
  ],
  top: 52,
  pins: { xs: [-20, -10, 0, 10, 20], top: 166, bottom: 190, width: 1.6 },
  getter: [52, 67],
  micas: [
    [80, 25],
    [142, 25],
  ],
  rods: [
    [-25, 76, 164],
    [0, 76, 164],
    [25, 76, 164],
  ],
  plates: [
    { rect: [-23, 84, 20, 54], ribs: [-13] },
    { rect: [3, 84, 20, 54], ribs: [13] },
  ],
  glows: [
    [-13, 80, 12],
    [13, 80, 12],
    [-13, 142, 11],
    [13, 142, 11],
  ],
};

/** The Mac's geometry table, one entry per family (0 noval triode .. 9 directly heated). */
const FAMILIES: Geometry[] = [
  NOVAL_TRIODE,
  // 1 noval pentode (EF86)
  {
    profile: [
      [28, 166],
      [28, 72],
    ],
    top: 52,
    pins: { xs: [-20, -10, 0, 10, 20], top: 166, bottom: 190, width: 1.6 },
    getter: [52, 67],
    micas: [
      [80, 25],
      [142, 25],
    ],
    rods: [
      [-22, 76, 164],
      [22, 76, 164],
    ],
    plates: [{ rect: [-17, 86, 34, 52], ribs: [-6, 6] }],
    glows: [
      [0, 81, 14],
      [0, 143, 12],
    ],
  },
  // 2 noval power (EL84)
  {
    profile: [
      [32, 168],
      [32, 58],
    ],
    top: 34,
    pins: { xs: [-22, -11, 0, 11, 22], top: 168, bottom: 192, width: 1.6 },
    getter: [34, 50],
    micas: [
      [60, 29],
      [144, 29],
    ],
    rods: [
      [-29, 56, 166],
      [29, 56, 166],
    ],
    plates: [{ rect: [-20, 66, 40, 72], fins: 5, ribs: [-7, 7] }],
    glows: [
      [0, 61, 16],
      [0, 144, 14],
    ],
  },
  // 3 octal glass (6SN7, 6SL7)
  {
    profile: [
      [30, 148],
      [30, 50],
    ],
    top: 26,
    base: { top: 146, bottom: 176, topHalf: 33, bottomHalf: 31, key: true },
    pins: { xs: [-21, -7, 7, 21], top: 176, bottom: 196, width: 2.6 },
    getter: [26, 41],
    micas: [
      [58, 27],
      [128, 27],
    ],
    rods: [
      [-26, 54, 146],
      [0, 54, 146],
      [26, 54, 146],
    ],
    plates: [
      { rect: [-22, 62, 19, 62], ribs: [-12.5] },
      { rect: [3, 62, 19, 62], ribs: [12.5] },
    ],
    glows: [
      [-12.5, 58, 12],
      [12.5, 58, 12],
      [-12.5, 128, 11],
      [12.5, 128, 11],
    ],
  },
  // 4 octal metal (6SJ7): no glow but a faint warmth where the can meets the base
  {
    profile: [
      [27, 150],
      [27, 60],
    ],
    top: 52,
    metal: true,
    base: { top: 148, bottom: 176, topHalf: 30, bottomHalf: 28, key: true },
    pins: { xs: [-19, -6.5, 6.5, 19], top: 176, bottom: 196, width: 2.6 },
    glows: [[0, 149, 9]],
    ring: [126, 132],
  },
  // 5 large octal power (EL34)
  {
    profile: [
      [34, 152],
      [34, 44],
    ],
    top: 14,
    base: { top: 150, bottom: 180, topHalf: 37, bottomHalf: 35, key: true },
    pins: { xs: [-24, -8, 8, 24], top: 180, bottom: 199, width: 2.8 },
    getter: [14, 31],
    micas: [
      [46, 31],
      [134, 31],
    ],
    rods: [
      [-30, 42, 150],
      [30, 42, 150],
    ],
    plates: [{ rect: [-23, 52, 46, 78], fins: 6, ribs: [0] }],
    glows: [
      [0, 47, 18],
      [0, 135, 16],
    ],
  },
  // 6 small octal power (6V6)
  {
    profile: [
      [27, 152],
      [27, 66],
    ],
    top: 44,
    base: { top: 150, bottom: 178, topHalf: 31, bottomHalf: 29, key: true },
    pins: { xs: [-20, -7, 7, 20], top: 178, bottom: 197, width: 2.6 },
    getter: [44, 59],
    micas: [
      [68, 24],
      [136, 24],
    ],
    rods: [
      [-24, 64, 150],
      [24, 64, 150],
    ],
    plates: [{ rect: [-17, 74, 34, 58], fins: 5, ribs: [0] }],
    glows: [
      [0, 69, 14],
      [0, 137, 12],
    ],
  },
  // 7 shouldered power (6L6, the "coke bottle")
  {
    profile: [
      [26, 152],
      [30, 140],
      [36, 116],
      [36, 104],
      [27, 76],
      [26, 60],
    ],
    top: 24,
    base: { top: 150, bottom: 180, topHalf: 34, bottomHalf: 32, key: true },
    pins: { xs: [-22, -7, 7, 22], top: 180, bottom: 199, width: 2.8 },
    getter: [24, 40],
    micas: [
      [66, 22],
      [136, 30],
    ],
    rods: [
      [-24, 62, 150],
      [24, 62, 150],
    ],
    plates: [{ rect: [-20, 72, 40, 60], fins: 7, ribs: [0] }],
    glows: [
      [0, 68, 16],
      [0, 137, 14],
    ],
  },
  // 8 beam bottle (KT88)
  {
    profile: [
      [30, 156],
      [32, 144],
      [42, 108],
      [42, 40],
    ],
    top: 8,
    base: { top: 154, bottom: 184, topHalf: 36, bottomHalf: 34, key: true },
    pins: { xs: [-24, -8, 8, 24], top: 184, bottom: 200, width: 2.8 },
    getter: [8, 25],
    micas: [
      [46, 39],
      [138, 36],
    ],
    rods: [
      [-34, 42, 154],
      [34, 42, 154],
    ],
    plates: [{ rect: [-26, 54, 52, 80], fins: 7, ribs: [-9, 9] }],
    glows: [
      [0, 49, 20],
      [0, 139, 16],
    ],
  },
  // 9 directly heated (300B): the filament glows, strung above the plate
  {
    profile: [
      [22, 162],
      [25, 152],
      [42, 118],
      [43, 98],
      [34, 56],
      [23, 36],
    ],
    top: 10,
    base: { top: 160, bottom: 188, topHalf: 36, bottomHalf: 34, key: false },
    pins: { xs: [-13, 13], top: 188, bottom: 200, width: 4.5 },
    getter: [10, 22],
    micas: [
      [50, 24],
      [140, 27],
    ],
    rods: [
      [-24, 48, 160],
      [24, 48, 160],
      [-7, 50, 57],
      [7, 50, 57],
    ],
    plates: [{ rect: [-19, 74, 38, 62], mesh: true }],
    filament: [
      [-15, 74],
      [-7, 57],
      [0, 74],
      [7, 57],
      [15, 74],
    ],
    glows: [[0, 66, 34]],
  },
];

export function geometryOf(family: number): Geometry {
  return FAMILIES[family] ?? NOVAL_TRIODE;
}

const f = (v: number) => Number(v.toFixed(2));

/** Catmull-Rom through `pts` as cubic segments, from the path's current point (Qt's `smooth`). */
function smooth(pts: [number, number][]): string {
  let d = "";
  for (let i = 0; i + 1 < pts.length; i++) {
    const p0 = pts[i === 0 ? 0 : i - 1]!;
    const p1 = pts[i]!;
    const p2 = pts[i + 1]!;
    const p3 = pts[Math.min(i + 2, pts.length - 1)]!;
    d += `C${f(p1[0] + (p2[0] - p0[0]) / 6)},${f(p1[1] + (p2[1] - p0[1]) / 6)} ${f(p2[0] - (p3[0] - p1[0]) / 6)},${f(p2[1] - (p3[1] - p1[1]) / 6)} ${f(p2[0])},${f(p2[1])}`;
  }
  return d;
}

/** The glass (or the can): the profile mirrored, a crown of two quarter ellipses, a pressed button at the foot. */
export function envelopePath(g: Geometry): string {
  const left = g.profile.map(([x, y]) => [-x, y] as [number, number]);
  const right = [...g.profile].reverse();
  const [lx, ly] = g.profile[g.profile.length - 1]!;
  const k = 0.5523;
  const rise = ly - g.top;
  const glassBottom = g.profile[0]![1];
  return (
    `M${f(left[0]![0])},${f(left[0]![1])}` +
    smooth(left) +
    `C${f(-lx)},${f(ly - rise * k)} ${f(-lx * k)},${f(g.top)} 0,${f(g.top)}` +
    `C${f(lx * k)},${f(g.top)} ${f(lx)},${f(ly - rise * k)} ${f(lx)},${f(ly)}` +
    smooth(right) +
    `Q0,${f(glassBottom + 5)} ${f(left[0]![0])},${f(left[0]![1])}Z`
  );
}

function halfWidth(g: Geometry, y: number): number {
  const pts = g.profile;
  for (let i = 0; i + 1 < pts.length; i++) {
    const [x0, y0] = pts[i]!;
    const [x1, y1] = pts[i + 1]!;
    if (y <= y0 && y >= y1) return x0 + (x1 - x0) * ((y0 - y) / Math.max(y0 - y1, 0.001));
  }
  return pts[pts.length - 1]![0];
}

const HEATER = "rgb(255 133 41";
const CORE = "rgb(255 199 115";
const white = (a: number) => `rgb(255 255 255 / ${a})`;
const black = (a: number) => `rgb(0 0 0 / ${a})`;
const grey = (v: number, a = 1) => `rgb(${Math.round(v * 255)} ${Math.round(v * 255)} ${Math.round(v * 255)} / ${a})`;

/** A horizontal gradient from x0 to x1 in the drawing's units. */
function Across(props: { id: string; x0: number; x1: number; stops: [number, string][] }) {
  return (
    <linearGradient id={props.id} gradientUnits="userSpaceOnUse" x1={props.x0} x2={props.x1} y1={0} y2={0}>
      {props.stops.map(([o, c]) => (
        <stop key={o} offset={o} stop-color={c} />
      ))}
    </linearGradient>
  );
}

/** The tube of `family`, its heater lit while `on`. */
export function TubePicture(props: { family: number; on: boolean }) {
  const id = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const g = geometryOf(props.family);
  const env = envelopePath(g);
  const w = Math.max(...g.profile.map((p) => p[0]));
  const dht = !!g.filament;
  const reflTop = g.profile[g.profile.length - 1]![1] - 2;
  const reflBottom = g.profile[0]![1] - 10;
  const refl = Array.from({ length: 13 }, (_, i) => {
    const y = reflTop + ((reflBottom - reflTop) * i) / 12;
    return `${i ? "L" : "M"}${f(-halfWidth(g, y) + 5)},${f(y)}`;
  }).join("");
  const filament = g.filament?.map((p) => p.join(",")).join(" ");
  const b = g.base;
  const r = 3.5;
  return (
    <svg class="fx-tube-picture" viewBox="-60 -2 120 206" role="img" aria-label="Tube" data-family={props.family} data-on={props.on ? "true" : "false"}>
      <defs>
        <radialGradient id={`${id}sh`}>
          <stop offset={0} stop-color={black(0.45)} />
          <stop offset={1} stop-color={black(0)} />
        </radialGradient>
        <linearGradient id={`${id}pin`} x1={0} x2={1} y1={0} y2={0}>
          <stop offset={0} stop-color={grey(0.45)} />
          <stop offset={0.5} stop-color={grey(0.85)} />
          <stop offset={1} stop-color={grey(0.5)} />
        </linearGradient>
        {b ? <Across id={`${id}base`} x0={-b.topHalf} x1={b.topHalf} stops={[[0, "rgb(23 18 15)"], [0.5, "rgb(64 48 38)"], [1, "rgb(28 20 18)"]]} /> : null}
        <Across id={`${id}glass`} x0={-w} x1={w} stops={[[0, white(0.16)], [0.2, white(0.04)], [0.65, white(0.02)], [0.92, white(0.1)], [1, white(0.14)]]} />
        <Across id={`${id}steel`} x0={-w - 3} x1={w + 3} stops={[[0, grey(0.3)], [0.28, grey(0.66)], [0.6, grey(0.48)], [1, grey(0.26)]]} />
        {g.getter ? (
          <linearGradient id={`${id}get`} gradientUnits="userSpaceOnUse" x1={0} x2={0} y1={g.getter[0]} y2={g.getter[1]}>
            <stop offset={0} stop-color={grey(0.88)} />
            <stop offset={0.45} stop-color={grey(0.62, 0.95)} />
            <stop offset={1} stop-color={grey(0.4, 0)} />
          </linearGradient>
        ) : null}
        <linearGradient id={`${id}sheen`} gradientUnits="userSpaceOnUse" x1={0} x2={0} y1={reflTop} y2={reflBottom}>
          <stop offset={0} stop-color={white(0.2)} />
          <stop offset={1} stop-color={white(0.02)} />
        </linearGradient>
        {(g.plates ?? []).map((p, i) => (
          <Across key={i} id={`${id}plate${i}`} x0={p.rect[0]} x1={p.rect[0] + p.rect[2]} stops={[[0, grey(0.2)], [0.33, grey(0.36)], [0.66, grey(0.24)], [1, grey(0.16)]]} />
        ))}
        {g.glows.map(([, , rad], i) => (
          <radialGradient key={i} id={`${id}glow${i}`}>
            <stop offset={0} stop-color={`${dht ? CORE : HEATER} / ${0.55 * (g.metal ? 0.45 : 1)})`} />
            <stop offset={0.45} stop-color={`${HEATER} / 0.2)`} />
            <stop offset={1} stop-color={`${HEATER} / 0)`} data-r={rad} />
          </radialGradient>
        ))}
        <clipPath id={`${id}env`}>
          <path d={env} />
        </clipPath>
      </defs>
      {/* The contact shadow on the shelf */}
      <ellipse cx={0} cy={g.pins.bottom + 0.5} rx={w * 0.9} ry={4} fill={`url(#${id}sh)`} />
      {g.pins.xs.map((x) => (
        <rect key={x} x={x - g.pins.width / 2} y={g.pins.top - 2} width={g.pins.width} height={g.pins.bottom - g.pins.top + 2} rx={g.pins.width / 2} fill={`url(#${id}pin)`} />
      ))}
      {b ? (
        <g>
          {b.key ? <rect x={-2.8} y={b.bottom - 2} width={5.6} height={9} rx={1.5} fill="rgb(41 31 26)" /> : null}
          <path
            d={`M${-b.topHalf},${b.top}L${b.topHalf},${b.top}L${b.bottomHalf},${b.bottom - r}Q${b.bottomHalf},${b.bottom} ${b.bottomHalf - r},${b.bottom}L${-b.bottomHalf + r},${b.bottom}Q${-b.bottomHalf},${b.bottom} ${-b.bottomHalf},${b.bottom - r}Z`}
            fill={`url(#${id}base)`}
            stroke={white(0.14)}
            stroke-width={0.7}
          />
          <line x1={-b.topHalf + 1} x2={b.topHalf - 1} y1={b.top + 1.2} y2={b.top + 1.2} stroke={white(0.18)} stroke-width={0.6} />
        </g>
      ) : null}
      {g.metal ? (
        <g>
          {g.ring ? <rect x={-w - 3} y={g.ring[0]} width={(w + 3) * 2} height={g.ring[1] - g.ring[0]} rx={2} fill={`url(#${id}steel)`} /> : null}
          <path d={env} fill={`url(#${id}steel)`} />
          {g.ring ? <rect x={-w - 3} y={g.ring[0]} width={(w + 3) * 2} height={g.ring[1] - g.ring[0]} rx={2} fill={`url(#${id}steel)`} stroke={black(0.35)} stroke-width={0.6} /> : null}
          <path d={env} fill="none" stroke={white(0.25)} stroke-width={0.8} />
          <rect x={-7} y={g.top - 3} width={14} height={4} rx={1.5} fill={grey(0.55)} stroke={black(0.3)} stroke-width={0.5} />
        </g>
      ) : (
        <g>
          <path d={env} fill={black(0.22)} />
          <path d={env} fill={`url(#${id}glass)`} />
          <g clip-path={`url(#${id}env)`}>
            {(g.rods ?? []).map(([x, y0, y1], i) => (
              <line key={`r${i}`} x1={x} x2={x} y1={y0} y2={y1} stroke={white(0.32)} stroke-width={0.8} />
            ))}
            {(g.plates ?? []).map((p, i) => {
              const [x, y, pw, ph] = p.rect;
              return (
                <g key={`p${i}`}>
                  {p.fins
                    ? [x - p.fins, x + pw].map((side) => <rect key={side} x={side} y={y + 4} width={p.fins} height={ph - 8} fill={grey(0.3)} stroke={white(0.18)} stroke-width={0.5} />)
                    : null}
                  <rect x={x} y={y} width={pw} height={ph} rx={1.2} fill={`url(#${id}plate${i})`} />
                  {p.mesh ? (
                    <g stroke={white(0.13)} stroke-width={0.45}>
                      {Array.from({ length: Math.ceil((pw + ph) / 3.2) }, (_, k) => {
                        const x0 = x - ph + k * 3.2;
                        return (
                          <g key={k}>
                            <line x1={x0} y1={y + ph} x2={x0 + ph} y2={y} />
                            <line x1={x0} y1={y} x2={x0 + ph} y2={y + ph} />
                          </g>
                        );
                      })}
                    </g>
                  ) : null}
                  {(p.ribs ?? []).map((rib) => (
                    <g key={rib}>
                      <line x1={rib} x2={rib} y1={y + 2} y2={y + ph - 2} stroke={black(0.45)} />
                      <line x1={rib + 0.9} x2={rib + 0.9} y1={y + 2} y2={y + ph - 2} stroke={white(0.14)} stroke-width={0.6} />
                    </g>
                  ))}
                  <rect x={x} y={y} width={pw} height={ph} rx={1.2} fill="none" stroke={white(0.22)} stroke-width={0.6} />
                </g>
              );
            })}
            {(g.micas ?? []).map(([y, half], i) => (
              <rect key={`m${i}`} x={-half} y={y - 0.9} width={half * 2} height={1.8} rx={0.9} fill={white(0.38)} />
            ))}
            {filament ? <polyline points={filament} fill="none" stroke={white(0.45)} stroke-width={0.9} stroke-linejoin="round" /> : null}
            {g.getter ? <rect x={-60} y={g.getter[0] - 2} width={120} height={g.getter[1] - g.getter[0] + 2} fill={`url(#${id}get)`} /> : null}
            <path d={refl} fill="none" stroke={`url(#${id}sheen)`} stroke-width={2.5} stroke-linecap="round" />
          </g>
          <path d={env} fill="none" stroke={white(0.5)} />
          <rect x={-2.6} y={g.top - 6} width={5.2} height={7.5} rx={2.6} fill={white(0.12)} stroke={white(0.5)} stroke-width={0.9} />
        </g>
      )}
      {/* The heater: a moment to glow, a little less to go dark */}
      <g class="fx-tube-heater" opacity={props.on ? 1 : 0} data-testid="fx-tube-heater">
        {g.metal ? null : <path d={env} fill={`${HEATER} / 0.05)`} />}
        {g.glows.map(([x, y, rad], i) => (
          <g key={i}>
            <circle cx={x} cy={y} r={rad} fill={`url(#${id}glow${i})`} />
            {dht || g.metal ? null : (
              <>
                <rect x={x - 2.3} y={y - 3.8} width={4.6} height={7.6} rx={2.3} fill={`${HEATER} / 0.45)`} />
                <rect x={x - 1.1} y={y - 2.6} width={2.2} height={5.2} rx={1.1} fill={`${CORE})`} />
              </>
            )}
          </g>
        ))}
        {filament ? (
          <g fill="none" stroke-linejoin="round" stroke-linecap="round">
            <polyline points={filament} stroke={`${HEATER} / 0.35)`} stroke-width={6.2} />
            <polyline points={filament} stroke={`${HEATER} / 0.35)`} stroke-width={4.2} />
            <polyline points={filament} stroke="rgb(255 230 179)" stroke-width={1} />
          </g>
        ) : null}
      </g>
    </svg>
  );
}

/** The type chips' groups (Qt's shelf): types 1..8, 9..10, 11..16. */
export const TUBE_GROUPS: { title: string; from: number; to: number }[] = [
  { title: "Preamp triodes", from: 1, to: 8 },
  { title: "Preamp pentodes", from: 9, to: 10 },
  { title: "Power stages", from: 11, to: 16 },
];

/** A type's chip text: its name before " / " ("12AX7 / ECC83" → "12AX7"). */
export function shortTubeName(name: string): string {
  return name.split(" / ")[0] ?? name;
}

/** The push-pull power types (Qt's `pushPull`): meant for use with the output stage. */
export function pushPull(type: number): boolean {
  return type >= 11 && type <= 15;
}
