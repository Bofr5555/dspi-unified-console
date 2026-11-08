// A modal question (WP WB6a): the browser's own modal <dialog>, so focus moves
// into it, the rest of the page is inert, and Escape cancels. The first button
// is the default action.

import type { ComponentChildren } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";

export interface DialogButton {
  label: string;
  /** Destructive (Clear, Reset): drawn as such. */
  danger?: boolean;
  onClick: () => void;
}

export function Dialog(props: {
  title: string;
  children?: ComponentChildren;
  buttons: DialogButton[];
  /** Escape, or the dialog closed by the browser. */
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (!d.open) d.showModal();
    return () => {
      if (d.open) d.close();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      class="main-dialog"
      aria-label={props.title}
      onCancel={(e) => {
        e.preventDefault();
        props.onCancel();
      }}
    >
      <h2>{props.title}</h2>
      {props.children}
      <div class="dialog-buttons">
        {props.buttons.map((b) => (
          <button type="button" key={b.label} class={b.danger ? "danger" : undefined} onClick={b.onClick}>
            {b.label}
          </button>
        ))}
      </div>
    </dialog>
  );
}

/** A name to change: the device cuts it to what it keeps, and the read-back shows that. */
export function RenameDialog(props: {
  title: string;
  label: string;
  initial: string;
  onSave: (name: string) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(props.initial);
  const save = () => props.onSave(name.trim());
  return (
    <Dialog
      title={props.title}
      onCancel={props.onCancel}
      buttons={[
        { label: "Rename", onClick: save },
        { label: "Cancel", onClick: props.onCancel },
      ]}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <label class="dialog-field">
          {props.label}
          <input
            name="name"
            value={name}
            autoFocus
            autoComplete="off"
            onInput={(e) => setName((e.target as HTMLInputElement).value)}
          />
        </label>
      </form>
    </Dialog>
  );
}
