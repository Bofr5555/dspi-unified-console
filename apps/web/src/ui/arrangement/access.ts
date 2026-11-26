// -- WP UB13 -- (UXP B13, D11) Outline controls and the control size (arrangement.ts): whether this
// is a touch screen, for Automatic, and the stylesheet that carries both (access.css).

import { useEffect, useState } from "preact/hooks";
import { comfortableFor, type Arrangement } from "./arrangement";
import "./access.css";

const COARSE = "(pointer: coarse)";

/** A coarse pointer (a finger) is the page's main one; follows a change (a tablet's keyboard taken off). */
export function useCoarsePointer(): boolean {
  const query = typeof matchMedia === "function" ? matchMedia(COARSE) : null;
  const [coarse, setCoarse] = useState(() => query?.matches ?? false);
  useEffect(() => {
    if (!query) return;
    const changed = () => setCoarse(query.matches);
    query.addEventListener("change", changed);
    changed();
    return () => query.removeEventListener("change", changed);
  }, []);
  return coarse;
}

/** The console's data-outline and data-density: nothing for the owner's look (off, Standard). */
export function accessAttributes(a: Pick<Arrangement, "outline" | "density">, coarse: boolean): Record<string, string | undefined> {
  return {
    "data-outline": a.outline === "on" ? "on" : undefined,
    "data-density": comfortableFor(a.density, coarse) ? "comfortable" : undefined,
  };
}
