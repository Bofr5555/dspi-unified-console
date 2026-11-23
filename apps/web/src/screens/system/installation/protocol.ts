// WP INSTW: the core's Installation setup calls (crates/dspi-wasm/src/installation.rs, WP INST0,
// MPLAN 1.4) as the page calls them. A draft is the page's: the core keeps none, and sends
// nothing but for the view and the save.

import type { SystemModel } from "../model";

/** A model being edited: as read at `revision` (`base`) and with the edits (`model`). */
export interface InstallationDraft {
  revision: number;
  base: SystemModel;
  model: SystemModel;
}

/** A DSPi the hub sees that the installation does not hold. */
export interface InstallationUnseen {
  serial: string;
  name: string;
  online: boolean;
  fw: string;
  outputs: number;
}

export interface InstallationView {
  revision: number;
  /** This session's role on the hub: `viewer`, `control`, `admin`. */
  role: string;
  /** The role may save (Admin). */
  mayEdit: boolean;
  draft: InstallationDraft;
  /** The units the hub sees that the installation does not hold, online first. */
  unseen: InstallationUnseen[];
  /** False for a hub without an inventory: `unseen` is then empty, not "none". */
  inventory: boolean;
}

/** One reason the hub would refuse a draft. The hub's validator stops at the first. */
export interface InstallationIssue {
  /** The element's id; "" for the installation as a whole. */
  subject: string;
  message: string;
}

export type InstallationRole = "receiver" | "repeater" | "spdif_follower" | "usb_direct";

// -- WP FEAT1 --
/** How a programme reaches the installation: what feeds a unit's inputs or a sender's channels. */
export type InstallationProgrammeKind = "computer_usb" | "spdif" | "analogue" | "alink_sender";

// -- WP FEAT1 --
/** One op of a scene; a `source` op switches a zone (target `zone:<id>`) to `source`. */
export interface InstallationSceneAction {
  target: string;
  op: string;
  arg?: number;
  value?: number;
  source?: string | null;
}

/** `dspi_core::link::InstallationEdit` as JSON (the edits the Installation tab makes). */
export type InstallationEdit =
  | { op: "addUnit"; serial: string; id?: string; name?: string; room?: string | null; role: InstallationRole; upstream?: string | null }
  | { op: "removeElement"; id: string }
  | { op: "renameElement"; id: string; name: string }
  | { op: "setUnitRoom"; id: string; room?: string | null }
  | { op: "setUnitRole"; id: string; role: InstallationRole }
  | { op: "setUnitUpstream"; id: string; upstream: string; output?: number | null }
  | { op: "addRoom"; id?: string; name: string }
  | { op: "addZone"; id?: string; name: string; rooms?: string[]; source?: string | null }
  | { op: "setZoneRooms"; id: string; rooms: string[] }
  | { op: "addSender"; id?: string; name: string; serial?: string | null }
  | { op: "addFanOut"; id?: string; name: string; serial?: string | null; upstream: string; output?: number | null }
  // -- WP WEBM3 -- an S/PDIF source's slots, groups
  | { op: "setSpdifSource"; id: string; txMask: number }
  | { op: "addGroup"; id?: string; name: string; units?: string[] }
  | { op: "setGroupUnits"; id: string; units: string[] }
  // -- WP FEAT1 -- slots, sources and what a zone plays, scenes, programme sources
  | { op: "setUnitSlots"; id: string; slots?: number[] | null }
  | { op: "addSource"; id?: string; name: string; sender?: string | null; channels?: number[] }
  | { op: "setSource"; id: string; sender?: string | null; channels: number[] }
  | { op: "setZoneSource"; id: string; source?: string | null }
  | { op: "addScene"; id?: string; name: string; actions?: InstallationSceneAction[]; systemMute?: boolean | null }
  | { op: "setScene"; id: string; actions: InstallationSceneAction[]; systemMute?: boolean | null }
  | { op: "addProgrammeSource"; id?: string; name: string; kind: InstallationProgrammeKind; feeds: string; inputs?: number[] }
  | { op: "setProgrammeSource"; id: string; kind: InstallationProgrammeKind; feeds: string; inputs: number[] };

export interface InstallationMethods {
  /**
   * The hub's installation as a draft, with the units it sees that it does not hold. Every
   * role may read it. Refusals: `not_open` (no hub), `link`.
   */
  installationView: { params: undefined; result: InstallationView };
  /** `edits` made on `draft`, all or none; sends nothing. `invalid` names the edit refused. */
  installationDraftEdit: { params: { draft: InstallationDraft; edits: InstallationEdit[] }; result: InstallationDraft };
  /** The hub's own validator on the draft: `[]` when the hub would take it. Sends nothing. */
  installationValidate: { params: { draft: InstallationDraft }; result: InstallationIssue[] };
  /** What saving would change, one sentence each. Sends nothing. */
  installationDiff: { params: { draft: InstallationDraft }; result: string[] };
  /**
   * One `system.set` at the draft's revision. Admin. Refusals: `conflict` (the installation
   * changed on the hub meanwhile: read it again), `denied`, `invalid` (nothing sent), `read_only`.
   */
  installationSave: { params: { draft: InstallationDraft }; result: { revision: number; warnings: string[] } };
}
