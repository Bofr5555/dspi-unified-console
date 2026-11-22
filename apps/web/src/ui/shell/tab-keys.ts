// -- WP WEBM1 -- The keys of a row of tabs (ARIA's tabs pattern, as the shell's tab rows use it):
// the arrows, Home and End move the focus and choose; the row is one Tab stop.

export function tabKeys(count: number, current: number, choose: (i: number) => void) {
  return (e: KeyboardEvent) => {
    if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey || !count) return;
    const at = Math.max(0, current);
    const to =
      e.key === "ArrowRight" ? (at + 1) % count : e.key === "ArrowLeft" ? (at - 1 + count) % count : e.key === "Home" ? 0 : e.key === "End" ? count - 1 : -1;
    if (to < 0) return;
    const row = e.currentTarget as HTMLElement;
    e.preventDefault();
    choose(to);
    row.querySelectorAll<HTMLElement>('[role="tab"]')[to]?.focus();
  };
}
