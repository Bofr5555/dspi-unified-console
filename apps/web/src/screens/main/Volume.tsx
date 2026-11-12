// The main window's volume (WP WB6a): the user volume or the master volume, as
// the Console's sidebar chooses (M47, remembered for this viewer), mute and
// bypass. Each control writes through the core and shows the value read back.
// A double click (or the secondary click, as on the Console) puts a slider back
// at 0 dB.

import { memo } from "preact/compat";
import { useState } from "preact/hooks";
import { safeLocalStorage } from "../../fleet/local";
import type { Session } from "../../ui/session";
import { useLiveWrite } from "./live";
import {
  MASTER_STEPS,
  USER_STEPS,
  masterToPos,
  posToMaster,
  posToUser,
  userToPos,
  volumeText,
} from "./model";

type Mode = "user" | "master";
const MODE_KEY = "dspi-web.volume-mode";

function savedMode(): Mode {
  try {
    return safeLocalStorage()?.getItem(MODE_KEY) === "master" ? "master" : "user";
  } catch {
    return "user";
  }
}

function saveMode(m: Mode): void {
  try {
    safeLocalStorage()?.setItem(MODE_KEY, m);
  } catch {
    // Not kept: the choice lasts this visit.
  }
}

export const VolumeCard = memo(function VolumeCard(props: {
  session: Session;
  userDb: number;
  masterDb: number;
  mute: boolean;
  bypass: boolean;
  disabled: boolean;
}) {
  const { session, disabled } = props;
  const [mode, setMode] = useState<Mode>(savedMode);
  const master = mode === "master";
  const path = master ? "vol.master" : "vol.user";
  const live = useLiveWrite((db) => session.write(path, { float: db }));
  const db = live.draft ?? (master ? props.masterDb : props.userDb);
  const toDb = master ? posToMaster : posToUser;
  const name = master ? "Master volume" : "Volume";
  const reset = (e: Event) => {
    e.preventDefault();
    if (!disabled) live.release(0);
  };

  return (
    <section class="card volume-card" aria-label="Volume">
      <div class="card-head">
        <select
          class="volume-mode"
          aria-label="Volume shown"
          value={mode}
          onChange={(e) => {
            const m = (e.target as HTMLSelectElement).value === "master" ? "master" : "user";
            setMode(m);
            saveMode(m);
          }}
        >
          <option value="user">Volume</option>
          <option value="master">Master volume</option>
        </select>
        <output class="volume-value" aria-live="polite">
          {volumeText(db, master)}
        </output>
      </div>
      <input
        type="range"
        class={master ? "volume-slider master" : "volume-slider"}
        aria-label={name}
        aria-valuetext={volumeText(db, master)}
        min={0}
        max={master ? MASTER_STEPS : USER_STEPS}
        step={1}
        value={master ? masterToPos(db) : userToPos(db)}
        disabled={disabled}
        onInput={(e) => live.move(toDb(Number((e.target as HTMLInputElement).value)))}
        onChange={(e) => live.release(toDb(Number((e.target as HTMLInputElement).value)))}
        onDblClick={reset}
        onContextMenu={reset}
      />
      <div class="toggles">
        <button
          type="button"
          class="toggle mute"
          aria-pressed={props.mute}
          disabled={disabled}
          onClick={() => void session.write("vol.mute", { bool: !props.mute })}
        >
          Mute
        </button>
        <button
          type="button"
          class="toggle bypass"
          aria-pressed={props.bypass}
          disabled={disabled}
          onClick={() => void session.write("bypass", { bool: !props.bypass })}
        >
          Bypass
        </button>
      </div>
    </section>
  );
});
