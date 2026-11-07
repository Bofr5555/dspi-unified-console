# Control surfaces on a phone: spike report (WP AN8)

What AN9–AN12 should build, how big they are, and what is still unknown.
Written from a working spike: the real core's demo device (`FakeDevice`, CG16)
behind the real `DspiBridgeImpl`, under a prototype Compose screen, in Robolectric
at 360×640 dp. No phone was used.

Reproduce: `git apply apps/android/docs/cs-spike/spike.patch`, then
`tools/dev.sh ./gradlew --no-daemon --console=plain :app:testDebugUnitTest --tests '*CsSpikeAn8Test*'`
from `apps/android`. 18 of 18 tests pass; each prints `AN8 ...` lines and writes
screenshots to `app/build/an8shots/`. The patch is not for merging. AN9 can start
from its harness (section 6).

## 1. The short version

- **Every feature fits. Nothing is cut**, as the Mac is the feature floor. What
  changes is depth: the desktop's expanding cards become pushed pages.
- **Sizes:** AN9 2–2.5, AN10 1, AN11 1, AN12 1.5–2, plus a new core WP of 1.75.
  Total 7.25–8.25, against the plan's 5.5–9.5 plus 0.5 for the demo (section 5).
- **Three things the plan did not know:**
  1. CG16 does not reach Android's demo mode. The app's demo is the Kotlin
     `FakeDspiCore`, whose control surfaces are a stub with no bindings. It is a
     small fix, shown working (section 4).
  2. The core exports the records but not the editing rules. They exist in
     `dspi_session::cs_model`, but no client outside Rust can call them. Porting means a
     fourth copy. I recommend exporting them (section 3).
  3. The demo offers 26 functions. Real firmware offers about 70 on a button, so a
     UI tested only on the demo will look better than it is (section 7, U1).

## 2. Screen map

```
Settings
├─ Control Surfaces      S1  list: name, what it does, Active pill. Add → S2
│   ├─ S2  Add: component type list (plain page)
│   ├─ S3  Control editor (button, switch, pot, encoder, LED, dimmable LED)
│   │       Function → P1. Channel/group, band → P2. GPIO → P3.
│   │       Behavior, gesture → menu (at most 6 items). Values → D1.
│   │       Toggles inline. Bar: Revert, Apply. Menu: Remove (asks), Change type.
│   ├─ S4  IR receiver: GPIO, sense, remote buttons list. Add → S5
│   │   └─ S5  Remote button: Learn → D2, then the same rows as S3
│   └─ S6  Display: wiring, status, settings, pages list → S7
│       └─ S7  One page: function, target, flags
├─ Channel Groups        S8  list → S9: name, kind, member chips, used by
├─ Macros                S10 list, each with Run → S11: name, step summaries with
│   │                        up/down, Add, Run → S12: one step
└─ Auxiliary Outputs     S13 list, each with its switch (and level) → S14: wiring,
                             timing, power-on, driven by

P1  Function picker: sectioned by the 16 families, with a search field
P2  Plain picker       P3  GPIO picker, taken pins dimmed with their owner
D1  Number entry, unit-aware       D2  Learn: 10 s, Cancel, screen kept on
```

The pages sit inside the existing `SettingsDetail`, so the header, Back and the
840 dp sidebar stay as they are. Above 840 dp the same stack runs in the detail pane.

Rules the pages keep from the Mac and Qt:

- **Two levels of commit.** Apply is live; Save writes flash. Same words, same gate
  (`CS_SAVE`). Failed Apply keeps the draft and shows why.
- **Add applies at once** with defaults and opens the editor. **Remove asks first.**
- **Run stays one tap** (macro fire, aux switch): instant and never a flash write.

## 3. The main decision: where the editing rules live

The snapshot gives the raw tables. The rules that decide what a card offers are
in three places, and none is reachable from Kotlin:

| Copy | Size | Note |
|---|---|---|
| `dspi_session::cs_model` (Terminal) | 2 942 lines before tests | The newest; row lists are in `dspi-tui/src/settings/surfaces.rs` |
| `CsHelper.qml` (Qt) | 1 023 lines | |
| Swift tab | about 1 300 of its 5 155 lines | |

They have already drifted. `binding_actions` in the session drops Hold on a long or
double press, because the firmware refuses it (`control_surfaces.c:1729`).
`CsHelper.validActions` still offers it, so Qt can draft a binding the device refuses.

| | Port to Kotlin | Export from the core (recommended) |
|---|---|---|
| New Kotlin | about 1 000 lines plus 500 of tests in AN9 | none for rules |
| Core | none | new WP, about 1.5 |
| Android total | 8–8.5 | 7.25–8.25 |
| Later | a fourth copy to keep in step | SW12 and Qt can drop theirs |

For Android alone it is close to break-even. The reason to do it is the plan's own
rule, "put logic in the core, and let a client add only UI", and SW12.

**Proposed WP (CG20), size 1.5.** `cs_describe(kind, slot, draft)` returns what a
page shows: the rows, function families, actions, targets, bands, pins with owners,
unit and range, labels and the summary phrase. `cs_edit(kind, slot, draft, change)`
returns the next draft, through `set_noun_in`, `default_operands` and
`sanitize_group_flags`. Add `cs_new_binding(slot, type)`, and the status text
(`explain_status`), which also replaces the table in `CsControllerImpl`. JSON, like
`cs_snapshot`; no device I/O, so it does not make the web harder. The row lists have to
move out of the Terminal to serve this, which is the part I am least sure of.

## 4. The demo device

*AN28 took option A: demo mode now runs on the real core, so the hybrid below is not needed
and `FakeDspiCore` is for tests only. The table is the spike's finding at the time.*

Android's demo mode: `startDemoSession` uses `FakeDspiCore.demo()`, whose
`csSnapshot` returns `"bindings": []`, no groups and no macros, and whose `csApply`
accepts anything. AN9's "runs on the demo device" would prove nothing on it, and the
Groups and Macros pages stay hidden.

| Option | Effort | Result |
|---|---|---|
| **B: hybrid (recommended now).** `DspiCoreApi by fake`, with the `cs*` calls going to a real core on `openDemo()` | about 30 lines in `:app` | Works: `an8HybridDemo` shows real surfaces on the Kotlin demo. Needs a notify forward for aux and learn, which the spike did not do |
| A: use the real demo for all of demo mode | separate WP, not sized | The scaffold renders on it (`an8TheAppScaffoldOnTheRealDemo`, `scaffold-rust-demo.png`); the other screens are unchecked. Fits "all clients from one core", so worth its own WP |
| C: extend the Kotlin fake | large | A second copy of `fake/surfaces.rs` (3 179 lines). No |

Other findings on it, through the FFI:

- **Refusals work and are worth testing.** Pin clash `{"ok":false,"status":26}`, shown on
  the page in `an8PrototypeFlow` (`proto-refused.png`). A bad index gives `error` text.
  Empty group 31, aux on a non-aux slot 38, `csDemoBusy` 27 on a write and a save.
- **Macros run**, a step per status read, ending at 255. The bridge's existing follower fits.
- **IR learn cannot finish through the FFI.** `learnArm` then `learnRead` stays at state 1
  until cancelled. `FakeRemote::cs_learned` is Rust-only. AN10 needs an export next to
  `cs_demo_busy` (`cs_demo_learn(protocol, code)`, size 0.25). The 10 s timeout is
  not modelled either.
- Snapshot: 16 KB, parsed in 6–19 ms on the JVM (phone unmeasured). Parsed on main today,
  per operation; acceptable, as slider drags use `sendOnly`.

## 5. Sizes

Basis: size 1 is about 1 000 lines with tests (AN3 1 105, QD2 856). Small WPs have
overrun that in lines (QD4 1 189 at 0.5, CG16 4 134 at 0.75), so I read these as
estimates with upward risk. Lines are estimates, from Swift and QML counts for the same work.

| WP | Plan | New | Holds | Kotlin, with tests |
|---|---|---|---|---|
| CG20 (new) | – | 1.5 | Section 3 | – |
| CG20a or in CG20 (new) | – | 0.25 | `cs_demo_learn` | – |
| AN9 | 2–4 | **2–2.5** | Demo hybrid, test wiring, draft store, S1–S3, P1–P3, D1 | 1 700–2 000 |
| AN10 | 1 | **1** | S4, S5, D2; **[HW]** | 600 |
| AN11 | 1–2 | **1** | S6, S7, polling; **[HW]** | 700 |
| AN12 | 1.5–2.5 | **1.5–2** | S8–S14 | 1 300–1 700 |
| **Total** | 5.5–9.5, +0.5 | **7.25–8.25** | | |

Without CG20: AN9 3.5, AN11 1.25, AN12 2–2.5, with `cs_demo_learn` still needed: total 8–8.5.

Order: CG20 before AN9. AN10–AN12 reuse AN9's record editor, so they follow it and can
then run in parallel. AN9's files should be split up front, since one file
per record kind is what lets those three not collide.

## 6. What suits a phone

Measured at 360×640 with `SettingRow`'s own insets. One screen is **488 dp**: 640 less
status bar 24, header 48, gesture bar 24 (assumed) and Apply bar 56. A 411×891 phone
gives about 250 dp more. The rows are the spike's port of the desktop's rows, title
and detail text included. Function families with a band add about 120 dp to the first three.

| Page | Height | Screens | Verdict |
|---|---|---|---|
| Button, Mute | 530 dp | 1.1 | Good |
| Encoder | 567 dp | 1.2 | Good; 903 dp (1.9) at font scale 1.5 |
| Pot, Output Gain | 697 dp | 1.4 | Good |
| Dimmable LED | 933 dp | 1.9 | Long but fine |
| Dimmable aux | 864 dp | 1.8 | Fine; live switch and level at the top |
| Display, 4 pages | 1 137 dp | 2.3 | Fine with pages on S7 |
| IR receiver, 4 buttons open inline | 2 120 dp | 4.3 | **No.** Hence S5 |
| Macro, 8 steps inline | 2 752 dp | 5.6 | **No.** Hence S12 (344 dp, 0.7) |

By feature:

- **Macro run, aux switch and level: the best fit.** A remote control in the hand. They are on the
  lists, first thing, and do not need an editor.
- **IR learn: a better fit than on a desktop.** Remote in one hand, screen in the other, at
  the device. D2 keeps the screen on and counts down.
- **Bindings: good, if pushed.** The desktop's expanding card loses the list position, and the
  card is taller than the screen. Editing at the bench beside the board suits it.
- **Function picker: needs the most care.** About 70 functions in 16 families on real
  firmware. `ChoiceRow`'s dropdown is not usable at that length; hence P1. It is
  also why the menus stop at 6 items.
- **Display pages: fine.** The panel itself is the preview.
- **Groups: fine.** Member chips wrap.
- **Macro editing: the worst fit, still workable.** Steps are summaries, up and down
  buttons rather than drag, as on the Mac.

Things that bite on a phone:

- **The save bar is about 130 dp tall** when dirty (read off `existing-dirty.png`). With the Apply
  bar, one screen falls from 488 to 358 dp. Show only Apply on S3–S14, and the bar on the lists.
- **Rotation recreates the activity** (no `configChanges` in the manifest). Drafts belong
  in the app-scoped `SettingsSaveCoordinator`, keyed by serial, not in the page.
- **Back with an edited draft asks.** System back needs its own handler: `SettingsView`'s closes
  the whole page.
- **A lost connection** keeps the draft readable, disables Apply and never replays (owner's rule).
- **Blocking:** every op runs on the bridge's one FFI thread; pages show busy, not a frozen row.

Bugs in the current page, all replaced by AN9:

- IR and Display rows read "Volume · Adjust" (`existing-controls.png`): a container has no function.
- The rename dialog's Remove clears the binding with no question, in the dismiss button's place.

## 7. Unknowns

| # | Unknown | Why it matters | Settled by |
|---|---|---|---|
| U1 | Pickers at real length. Demo: 26 of 79 functions, 23 on a button; firmware about 70 | P1's feel, and the only place the demo flatters | A test fixture with the full table in AN9; a look on hardware |
| U2 | Round-trip cost. Fetch is about 70 transfers, a group apply about 20 (it re-reads all 16 bindings), a macro apply 9 writes and re-reads. Demo timing (1–2 ms) says nothing | Over Link on a slow network a fetch could take seconds on the one FFI thread | AN13's phone test, adding CS timings |
| U3 | ESP32 bridge: `read_macro` asks 132 B in one transfer, and writes are at most 40 B. `LINK-HIL-CHECKLIST.md` says the *SET* is oversize, which looks stale. `CsData::read` tolerates failure, so macros might read as **empty** rather than as an error | Silent data loss on screen | A fake-hub test with `max_payload` 64 in AN12; XC4 |
| U4 | Robolectric has no keyboard, touch or real font metrics at the system's scale | Number entry and name fields with the keyboard up; predictive back | Emulator or phone pass in AN13 |
| U5 | IR on real receivers: protocols, window, repeat codes | AN10's D2 | **[HW]** |
| U6 | Real panels: models, I2C refusals, status polling cost | AN11 | **[HW]** |
| U7 | CG20's shape. The row lists live in the Terminal; moving them is unproven | Up to 0.5 more | CG20's first commit |
| U8 | Older firmware. V17–V31 is read-only (AN2), so editors do not open, but the lists on old caps are unchecked. `open_demo_wire` can test it | Low | AN9 test |

For the owner, and neither blocks AN9:

- **Detail lines.** The Mac's two-line details account for much of the height. Without them, a
  row is about 48 dp instead of 65, and the encoder fits on one small screen. The owner has called
  other text wordy. My default: keep details on the pages a row opens, and drop them from rows where the title
  is plain. The wording is shared, so this is a display choice only.
- **Demo option A,** whether to make it its own WP.

## 8. Other changes this asks for

- `WriteGuardedCore.CS_READ_OPS` has `refresh` and `refreshStatus` only. The core also treats
  `displayStatus`, `auxRead` and `learnRead` as reads, so they are refused on a read-only
  connection. Add them in AN11 and AN12.
- Qt: `CsHelper.validActions` offers Hold on a long or double press (section 3). A QD item, or gone with CG20.
- Test wiring: `:app` needs `testImplementation(libs.jna)`, `jna.library.path` and a dependency on
  `:ffi:cargoBuildHost`. For the pages' internals, tests should live in `:feature-settings`,
  which then needs Robolectric and `:ffi` as test dependencies, as in `:app`.
- Robolectric notes for AN9: `@GraphicsMode(NATIVE)` and `View.draw` give real screenshots with real text, already
  verified offline. The main looper must be pumped by hand (`shadowOf(...).idle()`), since `rule.waitUntil`
  does not with no content set; with the bridge's timers use `autoAdvance = false`, as AN27 does.
