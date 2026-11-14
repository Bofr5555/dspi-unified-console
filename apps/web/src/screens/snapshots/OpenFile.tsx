// WP P1WB1: offline files (P1OF) beside the device picker. Open File… opens a `.dspipreset`
// with no device, and New from Template… one of the templates this build ships; either opens
// as a session of its own that every screen edits, its strip saying "File: <name>". The
// template list is the core's, asked of the core that then holds the chosen template, so no
// Worker is started only to list.

import { useEffect, useRef, useState } from "preact/hooks";
import type { CoreClient } from "../../core/client";
import type { TemplateRecord } from "../../core/protocol";

export function OpenFiles(props: {
  spawnCore: () => CoreClient;
  /** Open `text` as a new session, on `core` when one was started for it. */
  onOpen: (text: string, fileName: string, core?: CoreClient) => void;
}) {
  const file = useRef<HTMLInputElement>(null);
  const [list, setList] = useState<{ core: CoreClient; templates: TemplateRecord[] } | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const held = useRef<CoreClient | null>(null);
  held.current = list?.core ?? null;
  // A list left open when the console goes away gives its Worker back.
  useEffect(() => () => void held.current?.dispose(), []);

  const open = async (input: HTMLInputElement) => {
    const f = input.files?.[0];
    input.value = "";
    if (f) props.onOpen(await f.text(), f.name);
  };

  const showTemplates = async () => {
    if (list) return;
    setBusy(true);
    const core = props.spawnCore();
    try {
      setList({ core, templates: await core.call("templateList", undefined) });
      setError("");
    } catch (e) {
      void core.dispose();
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const closeList = () => {
    if (list) void list.core.dispose();
    setList(null);
  };

  const choose = async (t: TemplateRecord) => {
    if (!list) return;
    try {
      const { text } = await list.core.call("templateText", { id: t.id });
      const core = list.core;
      setList(null);
      props.onOpen(text, `${t.name}.dspipreset`, core);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <section class="installation open-files" aria-label="Files">
      <h2>Files</h2>
      <ul>
        <li class="source">
          <span class="source-name">A configuration file, with no device</span>
          <button type="button" onClick={() => file.current?.click()}>
            Open File…
          </button>
          <input
            ref={file}
            type="file"
            hidden
            accept=".dspipreset,.json,application/json"
            aria-label="File to open"
            onChange={(e) => void open(e.target as HTMLInputElement)}
          />
        </li>
        <li class="source">
          <span class="source-name">A new system from a template</span>
          <button type="button" disabled={busy} aria-expanded={!!list} onClick={() => void (list ? closeList() : showTemplates())}>
            New from Template…
          </button>
        </li>
      </ul>
      {list ? (
        <ul class="template-list" aria-label="Templates">
          {list.templates.map((t) => (
            <li key={t.id} class="source">
              <span class="source-name">
                {t.name}
                <span class="note">
                  {" "}
                  · {t.platform}, {t.inputs} in, {t.outputs} out{t.status ? ` · ${t.status}` : ""}
                </span>
              </span>
              <button type="button" aria-label={`Open template ${t.name}`} onClick={() => void choose(t)}>
                Open
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      {error ? (
        <p class="error" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}
