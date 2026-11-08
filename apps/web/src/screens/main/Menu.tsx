// A menu button (WP WB6a): the Console's preset context menu and its Tools
// entries, in one button so a narrow screen does not grow a row of them. ARIA's
// menu button pattern: Enter or Space opens it on the first item, arrows move,
// Escape closes it and gives focus back, Tab leaves it.

import { useEffect, useRef, useState } from "preact/hooks";

export type MenuEntry = { label: string; disabled?: boolean; danger?: boolean; onSelect: () => void } | "separator";

export function Menu(props: { label: string; entries: MenuEntry[]; disabled?: boolean }) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLUListElement>(null);

  const items = () => Array.from(list.current?.querySelectorAll<HTMLButtonElement>("[role=menuitem]:not([disabled])") ?? []);
  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  };

  useEffect(() => {
    if (!open) return;
    items()[0]?.focus();
    const outside = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!list.current?.contains(t) && !button.current?.contains(t)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);

  const onKey = (e: KeyboardEvent) => {
    if (!open) return;
    const all = items();
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const go = (i: number) => all[(i + all.length) % all.length]?.focus();
    if (e.key === "ArrowDown") go(at + 1);
    else if (e.key === "ArrowUp") go(at - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(all.length - 1);
    else if (e.key === "Escape") close(true);
    else if (e.key === "Tab") return setOpen(false);
    else return;
    e.preventDefault();
  };

  return (
    // Keys on the whole menu: an Escape before the first item has focus still closes it.
    <div class="menu" onKeyDown={onKey}>
      <button
        ref={button}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={props.disabled}
        onClick={() => setOpen(!open)}
      >
        {props.label} <span aria-hidden="true">▾</span>
      </button>
      {open ? (
        <ul ref={list} role="menu" aria-label={props.label}>
          {props.entries.map((m, i) =>
            m === "separator" ? (
              <li key={`sep${i}`} role="separator" />
            ) : (
              <li key={m.label} role="none">
                <button
                  type="button"
                  role="menuitem"
                  tabIndex={-1}
                  class={m.danger ? "danger" : undefined}
                  disabled={m.disabled}
                  onClick={() => {
                    close(true);
                    m.onSelect();
                  }}
                >
                  {m.label}
                </button>
              </li>
            ),
          )}
        </ul>
      ) : null}
    </div>
  );
}
