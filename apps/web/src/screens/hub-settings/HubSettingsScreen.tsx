// WP P2WB2: the hub's settings, a page of its own (review S7: the hub page links to it by its id,
// `hub-settings`). Shown when the session's hub has one of these families (`hubCaps`, no I/O), so
// an upstream hub without them never shows it. Sections, each by its cap:
//
// - Alarm rules (`alarms`): on or off and the thresholds per code, "mute on clip storm".
// - Schedules (`schedule`): the rules with their next and last runs, an editor, Run now.
// - Notifications (`outbound`): webhook and push targets, Test, the secret shown once.
// - MQTT and Home Assistant (`mqtt`): the broker, its TLS, the Home Assistant switch and key.
// - API keys (`api_keys`): the list, Mint (the key shown once), Revoke.
//
// Every call is the core's, in the Worker. Only an Admin of the hub changes anything: for a Control
// or Viewer session the page is read-only with the core's row `installer.hub_settings_role`, and the
// Admin-only reads show the hub's refusal as it answers.

import { useEffect, useState } from "preact/hooks";
import type { HubApiKeyRecord, HubCapsRecord } from "../../core/hub-settings-protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenDef, ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { NoticeRow, failure } from "../installer/HubAdmin";
import { Problem, type SectionProps } from "./parts";
import { AlarmRules } from "./AlarmRules";
import { Mqtt } from "./Mqtt";
import { Outbound } from "./Outbound";
import { Schedules } from "./Schedules";
import { HUB_SETTINGS_ROLE_KEY, HUB_SETTINGS_SCREEN, KEY_ROLES, localTime, mayEdit, mintedFrom, roleWord, sectionsOf, type MintedKey } from "./model";

/** The hub's caps and this session's role on it, read again when the Link phase changes. */
export function useHubCaps(session: Session, phase: string): { caps: HubCapsRecord | null; role: string } {
  const [caps, setCaps] = useState<HubCapsRecord | null>(null);
  const [role, setRole] = useState("");
  useEffect(() => {
    let live = true;
    void Promise.all([
      session.core.call("hubCaps", undefined).catch(() => null),
      session.core.call("hubRole", undefined).catch(() => ({ role: "" })),
    ]).then(([c, r]) => {
      if (!live) return;
      setCaps(c);
      setRole(r.role);
    });
    return () => {
      live = false;
    };
  }, [session, phase]);
  return { caps, role };
}

export function HubSettingsScreen(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const { caps, role } = useHubCaps(session, st.link?.phase ?? "");
  const sections = sectionsOf(caps);
  const ro = session.readOnlyReason;
  const edit = mayEdit(role, ro);
  const serial = st.snapshot?.device.serial || session.deviceId || "";
  const p: SectionProps = { session, edit, ask: props.ask, serial };
  return (
    <div class="hub-settings-screen">
      <h2 class="hub-settings-title">Hub Settings</h2>
      {/* -- HOOK SECWEB (PLAN-SEC SEC-C) -- the hub's TLS fingerprint and the "unencrypted" banner go here */}
      {/* -- HOOK SECWEB (PLAN-SEC SEC-C) -- the hub's upstream-compatibility mode (owner-approved: off by default,
          a large warning that security is downgraded, listing what turns off) goes here, Admin only */}
      {role && role !== "admin" ? <NoticeRow rowKey={HUB_SETTINGS_ROLE_KEY} label="Read-only" cls="read-only" /> : null}
      {role === "admin" && ro ? (
        <p class="note read-only" role="note" aria-label="Read-only">
          {ro}
        </p>
      ) : null}
      {role ? <p class="note hub-role">This console's role on the hub: {roleWord(role)}.</p> : null}
      {sections.includes("alarms") ? <AlarmRules {...p} /> : null}
      {sections.includes("schedule") ? <Schedules {...p} /> : null}
      {sections.includes("outbound") ? <Outbound {...p} /> : null}
      {sections.includes("mqtt") ? <Mqtt {...p} keys={sections.includes("apiKeys")} /> : null}
      {sections.includes("apiKeys") ? <ApiKeys {...p} /> : null}
      {caps && !sections.length ? <p class="note">This hub has no settings of its own to show here.</p> : null}
    </div>
  );
}

/** The page's entry among the session's screens (screens.tsx adds it with `withHubSettings`). */
export const HUB_SETTINGS_DEF: ScreenDef = { id: HUB_SETTINGS_SCREEN, label: "Hub Settings", View: HubSettingsScreen };

// ---------------------------------------------------------------- API keys

/** The hub's API keys: the list, Mint (shown once), Revoke. Admin only (the hub refuses below). */
function ApiKeys(props: SectionProps) {
  const { session, edit } = props;
  const [keys, setKeys] = useState<HubApiKeyRecord[] | null>(null);
  const [problem, setProblem] = useState("");
  const [name, setName] = useState("");
  const [role, setRole] = useState<string>("control");
  const [minted, setMinted] = useState<MintedKey | null>(null);
  const [busy, setBusy] = useState(false);
  const read = async () => {
    try {
      setKeys(await session.core.call("hubApiKeys", undefined));
      setProblem("");
    } catch (e) {
      setKeys(null);
      setProblem(failure(e));
    }
  };
  useEffect(() => {
    void read();
  }, [session]);
  const mint = async () => {
    setBusy(true);
    try {
      // Wrapped at once: the key lives only in this card until Done.
      setMinted(mintedFrom(await session.core.call("hubApiKeyMint", { name: name.trim(), role })));
      setName("");
      await read();
    } catch (e) {
      setProblem(failure(e));
    } finally {
      setBusy(false);
    }
  };
  const revoke = async (k: HubApiKeyRecord) => {
    const a = await props.ask({
      title: "Revoke API Key",
      body: <p>Revoke “{k.name}”? Whatever uses it is refused from now on.</p>,
      buttons: [
        { label: "Revoke", value: "revoke", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a !== "revoke") return;
    try {
      await session.core.call("hubApiKeyRevoke", { cid: k.cid });
      await read();
    } catch (e) {
      setProblem(failure(e));
    }
  };
  return (
    <section class="card hub-section" aria-label="API keys">
      <h3>API keys</h3>
      <p class="note">An API key lets a script or a home automation system use the hub's REST mirror with the role it was given.</p>
      <Problem text={problem} />
      {minted ? (
        <div class="card limits-report warn hub-secret" role="status" aria-label="New API key">
          <strong>The key for “{minted.name}” ({roleWord(minted.role)})</strong>
          <p class="recovery-code">
            <code>{minted.key.reveal()}</code>
          </p>
          <p class="note">It is shown only now: the hub keeps only a hash of it. Copy it to where it is used.</p>
          <button type="button" onClick={() => setMinted(null)}>
            Done
          </button>
        </div>
      ) : null}
      {keys ? (
        keys.length ? (
          <ul class="snap-list" aria-label="API keys on the hub">
            {keys.map((k) => (
              <li key={k.cid} class="snap-row" aria-label={k.name}>
                <div class="snap-what">
                  <strong>{k.name}</strong> <span class="badge">{roleWord(k.role)}</span>
                  <span class="note">
                    {k.created ? ` Made ${localTime(k.created)}` : ""}
                    {k.lastSeen ? ` · last used ${localTime(k.lastSeen)}` : " · never used"}
                    {k.online ? " · in use now" : ""}
                  </span>
                </div>
                <div class="snap-actions">
                  <button type="button" class="danger-text" disabled={!edit} aria-label={`Revoke ${k.name}`} onClick={() => void revoke(k)}>
                    Revoke
                  </button>
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <p class="note">No API keys yet.</p>
        )
      ) : null}
      <form
        class="power-rows"
        onSubmit={(e) => {
          e.preventDefault();
          void mint();
        }}
      >
        <label class="limit-field">
          <span>Key name</span>
          <input value={name} maxLength={64} disabled={!edit || busy} onInput={(e) => setName((e.target as HTMLInputElement).value)} />
        </label>
        <label class="limit-field">
          <span>Role</span>
          <select aria-label="Key role" value={role} disabled={!edit || busy} onChange={(e) => setRole((e.target as HTMLSelectElement).value)}>
            {KEY_ROLES.map((r) => (
              <option key={r} value={r}>
                {roleWord(r)}
              </option>
            ))}
          </select>
        </label>
        <div class="installer-buttons">
          <button type="submit" disabled={!edit || busy || !name.trim()}>
            Mint Key
          </button>
        </div>
      </form>
    </section>
  );
}
