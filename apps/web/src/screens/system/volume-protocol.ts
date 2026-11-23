// WP FEATUI: a room's or zone's volume (FEAT2, crates/dspi-wasm/src/system_volume.rs) as the
// page calls it: plan (reads each unit, writes nothing), then set the plan.

/** What to set: one of `room` and `zone` (an id), one of `levelDb` and `stepDb`. */
export interface SystemVolumeAsk {
  room?: string;
  zone?: string;
  volume?: "user" | "master";
  levelDb?: number;
  stepDb?: number;
}

/** One unit of the room or zone. `state`: ready (would be written), unchanged, refused, done. */
export interface SystemVolumeUnit {
  unitId: string;
  name: string;
  serial: string;
  beforeDb?: number | null;
  targetDb?: number | null;
  afterDb?: number | null;
  louder: boolean;
  state: "ready" | "unchanged" | "refused" | "done";
  code: string;
  message: string;
}

export interface SystemVolumePlan {
  ask: SystemVolumeAsk;
  name: string;
  number: number;
  of: number;
  units: SystemVolumeUnit[];
}

export interface SystemVolumeMethods {
  /** Each unit's volume now and what it would be; nothing written. Refusals: `invalid`, `not_open`, `link`. */
  systemVolumePlan: { params: SystemVolumeAsk; result: SystemVolumePlan };
  /** The plan's ready units written, each row settled. */
  systemVolumeSet: { params: { plan: SystemVolumePlan }; result: SystemVolumePlan };
}
