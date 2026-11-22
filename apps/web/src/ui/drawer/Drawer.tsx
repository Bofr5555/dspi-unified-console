// -- WP WEBM4 -- The notification drawer, from the top bar's bell (the desktop's NotificationDrawer,
// LAY3): what waits on the open devices, each with Show (its tab) and Dismiss, and what was said
// earlier on this page. It lies over the page's right side under the top bar (fixed in the window,
// at the console's right edge: a narrow page's top bar scrolls away); a press beside it or Escape
// closes it, and the focus goes back to the bell. The status strip keeps what limits what can
// be done (WEBM1); a device's error and step notice show there too, as before.

import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import type { Session } from "../session";
import { waiting, type NoticeLog } from "./notices";

/** The bell's count and the drawer's contents, current. */
export function useNotices(log: NoticeLog, sessions: readonly Session[]) {
  const [, bump] = useState(0);
  useEffect(() => log.watch(() => bump((n) => n + 1)), [log]);
  return { waiting: waiting(sessions), earlier: log.earlier };
}

export function BellButton(props: { count: number; open: boolean; onClick: () => void; buttonRef: { current: HTMLButtonElement | null } }) {
  const name = props.count ? `Notifications, ${props.count} waiting` : "Notifications";
  return (
    <button ref={props.buttonRef} type="button" class="bell" aria-label={name} title={name} aria-expanded={props.open} aria-haspopup="dialog" onClick={props.onClick}>
      <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" />
        <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
      </svg>
      {props.count ? (
        <span class="bell-count" aria-hidden="true">
          {props.count > 9 ? "9+" : props.count}
        </span>
      ) : null}
    </button>
  );
}

const time = (d: Date) => d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });

export function Drawer(props: {
  log: NoticeLog;
  sessions: readonly Session[];
  /** Show a device's tab. */
  onShow: (s: Session) => void;
  /** System › Events of the device on show, when it has that page. */
  onEvents: (() => void) | null;
  onClose: (refocus: boolean) => void;
  bell: { current: HTMLButtonElement | null };
}) {
  const { waiting: now, earlier } = useNotices(props.log, props.sessions);
  const sheet = useRef<HTMLElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  // Under the bell's row, at the console's right edge, inside the window
  const place = () => {
    const el = sheet.current;
    const bell = props.bell.current;
    if (!el || !bell) return;
    const row = (bell.closest(".top") ?? bell).getBoundingClientRect();
    const console_ = (bell.closest(".dspi-web") ?? document.body).getBoundingClientRect();
    const top = Math.max(0, Math.min(row.bottom, window.innerHeight - 160));
    el.style.top = `${top}px`;
    el.style.right = `${Math.max(0, window.innerWidth - console_.right)}px`;
    el.style.maxHeight = `${window.innerHeight - top - 8}px`;
  };
  useLayoutEffect(() => {
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, []);
  useEffect(() => {
    close.current?.focus({ preventScroll: true });
    const outside = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!sheet.current?.contains(t) && !props.bell.current?.contains(t)) props.onClose(false);
    };
    document.addEventListener("pointerdown", outside, true);
    return () => document.removeEventListener("pointerdown", outside, true);
  }, []);
  return (
    <section
      ref={sheet}
      class="drawer"
      role="dialog"
      aria-modal="false"
      aria-label="Notifications"
      onKeyDown={(e) => {
        if (e.key !== "Escape") return;
        e.preventDefault();
        e.stopPropagation();
        props.onClose(true);
      }}
    >
      <div class="drawer-head">
        <h2>Notifications</h2>
        {props.onEvents ? (
          <button
            type="button"
            onClick={() => {
              props.onClose(false);
              props.onEvents!();
            }}
          >
            Events…
          </button>
        ) : null}
        <button ref={close} type="button" class="drawer-close" aria-label="Close" title="Close" onClick={() => props.onClose(true)}>
          <span aria-hidden="true">×</span>
        </button>
      </div>
      {now.length ? (
        <ul class="drawer-list" aria-label="Waiting">
          {now.map((w) => (
            <li key={w.key} class={`drawer-item drawer-${w.kind}`}>
              <span class="drawer-device">{w.device}</span>
              <span class="drawer-text">{w.text}</span>
              <span class="drawer-actions">
                <button
                  type="button"
                  onClick={() => {
                    props.onClose(false);
                    props.onShow(w.session);
                  }}
                >
                  Show
                </button>
                <button type="button" onClick={w.dismiss}>
                  Dismiss
                </button>
              </span>
            </li>
          ))}
        </ul>
      ) : (
        <p class="drawer-empty">Nothing waits for you.</p>
      )}
      {earlier.length ? (
        <>
          <div class="drawer-head drawer-earlier">
            <h3>Earlier</h3>
            <button type="button" onClick={() => props.log.clear()}>
              Clear
            </button>
          </div>
          <ul class="drawer-list" aria-label="Earlier">
            {earlier.map((n) => (
              <li key={n.id} class={`drawer-item drawer-${n.kind}`}>
                {n.device ? <span class="drawer-device">{n.device}</span> : null}
                <span class="drawer-text">{n.text}</span>
                <time class="drawer-time" dateTime={n.at.toISOString()}>
                  {time(n.at)}
                </time>
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </section>
  );
}
