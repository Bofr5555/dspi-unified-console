// WP SECWEB (PLAN-SEC 4.6; owner rule 2026-11-15): a hub link's security, shown.
//
// - The connection bar: on a hub link that is not encrypted (a hub without TLS, an
//   upstream hub, compatibility mode) the bar is red and its text ends with
//   " (unsecured)", so it is said in words, not by colour alone; beside it a small
//   info button opens the core's calm explanation (`dlink.unsecured.info`), with what
//   to do where the hub can encrypt (its https page) or that its compatibility mode is
//   on (`dlink.unsecured.compat`).
// - The hub's settings: the hub key's fingerprint to compare with the hub's display, or
//   the "unencrypted" banner; and, for an Admin, the large downgrade warning when the
//   hub's upstream compatibility mode is on (`dlink.upstream_compat`).
//
// All words but the fingerprint's own come from the core's notice table.

import { useState } from "preact/hooks";
import { Dialog } from "../screens/main/Dialog";
import { httpsConsoleOf, type LinkContext, type LinkSecurity } from "../fleet/security";
import { noticeRow } from "./texts";

export const UNSECURED_KEY = "dlink.unsecured";
export const UNSECURED_INFO_KEY = "dlink.unsecured.info";
export const UNSECURED_COMPAT_KEY = "dlink.unsecured.compat";
export const HUB_COMPAT_KEY = "dlink.upstream_compat";

/** Whether the bar turns red: a hub link known to be unencrypted. */
export function isUnsecured(sec: LinkSecurity | null | undefined): boolean {
  return !!sec && sec.state === "unsecured";
}

/** The bar's suffix, " (unsecured)", in the core's word. */
export function unsecuredSuffix(): string {
  return ` (${noticeRow(UNSECURED_KEY)?.title || "unsecured"})`;
}

/** " (unsecured)" and its info button, after the connection text; nothing when encrypted. */
export function UnsecuredMark(props: { security: LinkSecurity | null | undefined; ctx?: LinkContext | null }) {
  const [open, setOpen] = useState(false);
  if (!isUnsecured(props.security)) return null;
  return (
    <>
      <span class="link-unsecured-text">{unsecuredSuffix()}</span>
      <button type="button" class="link-info" aria-label="About this connection" title="About this connection" onClick={() => setOpen(true)}>
        i
      </button>
      {open ? <UnsecuredInfo security={props.security!} ctx={props.ctx ?? null} onClose={() => setOpen(false)} /> : null}
    </>
  );
}

/** The explanation: calm, what it means, what to do. */
export function UnsecuredInfo(props: { security: LinkSecurity; ctx: LinkContext | null; onClose: () => void }) {
  const info = noticeRow(UNSECURED_INFO_KEY);
  const compat = props.security.upstreamCompat ? noticeRow(UNSECURED_COMPAT_KEY) : null;
  const https = props.ctx ? httpsConsoleOf(props.ctx, props.security) : "";
  return (
    <Dialog title={info?.title || "About this connection"} onCancel={props.onClose} buttons={[{ label: "Close", onClick: props.onClose }]}>
      <div class="link-unsecured-info" data-key={UNSECURED_INFO_KEY}>
        <p>{info?.reason}</p>
        {compat ? (
          <p data-key={UNSECURED_COMPAT_KEY}>
            <strong>{compat.title}.</strong> {compat.reason} {compat.action}
          </p>
        ) : https ? (
          <p class="link-https">
            This hub also serves an encrypted page: <a href={https}>{https}</a>
          </p>
        ) : (
          <p class="notice-action">{info?.action}</p>
        )}
      </div>
    </Dialog>
  );
}

/** The hub's settings: its key's fingerprint when encrypted, else the "unencrypted" banner. */
export function HubLinkSecurity(props: { security: LinkSecurity | null | undefined; ctx?: LinkContext | null }) {
  const sec = props.security;
  if (!sec) return null;
  if (sec.state === "encrypted") {
    return (
      <section class="card hub-section hub-tls" aria-label="Connection security">
        <h3>Connection security</h3>
        <p class="note">
          Encrypted{sec.via === "proxy" ? " (through this computer's uconsole web, checked against the key it trusts)" : ""}.
        </p>
        {sec.hubKey ? (
          <>
            <p class="hub-fingerprint">
              Hub key: <code>{sec.hubKey}</code>
            </p>
            <p class="note">Check this matches the key your hub shows (uconsole hub tls fingerprint, or its window).</p>
          </>
        ) : null}
      </section>
    );
  }
  if (sec.state === "local") return null;
  const info = noticeRow(UNSECURED_INFO_KEY);
  const https = props.ctx ? httpsConsoleOf(props.ctx, sec) : "";
  return (
    <div class="card hub-unencrypted notice-warning" role="note" aria-label="Unencrypted" data-key={UNSECURED_INFO_KEY}>
      <strong>Unencrypted.</strong> {info?.reason}{" "}
      {https ? (
        <span>
          Use the encrypted page: <a href={https}>{https}</a>
        </span>
      ) : (
        <span class="notice-action">{info?.action}</span>
      )}
    </div>
  );
}

/** The large downgrade warning, for an Admin, while the hub's upstream compatibility mode is on. */
export function UpstreamCompatWarning(props: { security: LinkSecurity | null | undefined; admin: boolean }) {
  if (!props.admin || props.security?.upstreamCompat !== true) return null;
  const n = noticeRow(HUB_COMPAT_KEY);
  if (!n) return null;
  return (
    <div class="card hub-compat-warning notice-warning" role="alert" aria-label="Security downgraded" data-key={HUB_COMPAT_KEY}>
      <h3>{n.title}</h3>
      <p>{n.reason}</p>
      <p class="notice-action">{n.action}</p>
    </div>
  );
}
