# Tracking upstream Swift Console

*2026-11-07, SW-U1.* How `apps/apple` keeps following Weeb Labs' Swift DSPi Console, which changes
this repository may make to it, and how an upstream release gets merged. The rule is in
[`core-merge/PLAN-CLIENTS.md`](core-merge/PLAN-CLIENTS.md) section 0.1; this file is its procedure.
Measured on `main` at `e6db6e70`.

## 1. The repositories

| Name | Where | Role |
|---|---|---|
| Upstream | `https://github.com/WeebLabs/DSPi-Console` | Weeb Labs' app. Releases are cut on `release/vX.Y.Z` branches; `main` lags them (on 2026-11-07 `main` is at `fc686ab`, 2026-06-13, and `release/v1.1.6` at `9a42965`, 2026-11-02) |
| Fork | `https://github.com/Bofr5555/DSPi-Console` | The owner's fork. `ext-channel-116` at `48cf7c5` was imported by MR1; the other `-116` branches stay there. It is the mirror that follows upstream (MR1) |
| This repo | `apps/apple/` | The import, with history, under a path prefix |

MR1 rewrote every commit with `git filter-branch --index-filter` so that its tree sits under
`apps/apple/`, keeping authors, committers, dates and messages. **That rewrite is deterministic:**
rewriting upstream again the same way gives the same hashes that are already in this history.
Checked: upstream `main` `fc686ab` rewrites to `77b9fc95`, and `release/v1.1.6` `9a42965` to
`41d9d469`; both are commits of `main` here (`git merge-base --is-ancestor` succeeds). So an
upstream branch, once rewritten, shares real ancestors with `main`, and an ordinary `git merge`
finds the right base. No subtree merge or hand-made graft is needed.

## 2. The rule

The diff of `apps/apple` against upstream may only be in these places:

1. **`DSPi Console/Core/`**: `CoreBackedTransport.swift` and `Core/Generated/` (the UniFFI bindings and
   `DspiConstants.swift`, generated, never edited by hand).
2. **The removal of the command layer:** `Commands.swift`, and the other wire functions that
   [`SWIFT-SEAMS.md`](SWIFT-SEAMS.md) section 2 lists (`ExtensionCommands.swift`, the transfers in
   `StatsView.swift`, `SpectrumAnalyser.swift`, `InterruptMonitor.swift`, `PresetDocumentTransfer.swift`,
   `FirmwareUpdateView.swift`), as SW4-SW14 replace them with core calls.
3. **View call sites** that switch from a command function to the core: small edits, marked
   `// -- WP SWn --`, as in `DSPi_ConsoleApp.swift` today.
4. **Build and test plumbing** that upstream has no counterpart for: `build-xcframework.sh`,
   `ci-test.sh`, `Tools/gen_swift_constants.py`, `TestHostGuard.swift`, test files carrying an SW id,
   `.gitignore`, and the `project.pbxproj` entries for those files.

**Any other change needs a reason in its review.** The reviewer checks it with:

```
git diff --stat <rewritten upstream release> main -- apps/apple
```

and compares the list with the four places above and the fork layer below.

**The fork layer.** The import brought the owner's own Swift work from the fork, which upstream does
not have: USB MIDI transport, the extension channel, the iOS target, Link pairing on iOS, crossfeed
pair map, core-voltage status (78 files, +9 518 / -84 against upstream `release/v1.1.6`). It is not
core-merge work and the rule does not ask to undo it. It shrinks as the core takes over the wire code
(the MIDI transport and extension commands are wire code, so they go with point 2). New work in this
layer also needs a reason in its review.

**Today** (`main` at `e6db6e70`), the core-merge part of the diff (`09e16a31` to `e6db6e70`) is 18 files,
+25 311 / -24, of which 22 080 lines are the generated bindings. Outside points 1-4: nothing.
`Commands.swift` is not removed yet: SW4 put `CoreBackedTransport` behind `-CoreTransport YES`, and the
command functions still run on both paths.

## 3. Merging an upstream release

Each upstream release gets its own merge WP, named `SW-UP<n>`, size 0.5-1 (PLAN-CLIENTS section 0.1).

**Owns:** the merge commit, conflict resolutions in `apps/apple`, and the follow-up list (below).
**Done when:** the merge commit's second parent is the rewritten release tip; the macOS and iOS test
schemes pass in `macvm` as they did before the merge (same known failures, MR1's list); the follow-up
list is in the report.

Steps, all under `~/.cache/dspi-build/work/wp-<id>/`, never in the main checkout:

```sh
# 1. Fetch upstream into a scratch clone. Never fetch it into this repo unrewritten.
git clone https://github.com/WeebLabs/DSPi-Console upstream
cd upstream
git branch -f t-rel origin/release/vX.Y.Z

# 2. Rewrite it exactly as MR1 did. Any other filter (git filter-repo, another prefix spelling,
#    keeping signatures) gives other hashes and loses the common history.
FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch -f --index-filter \
  'git read-tree --empty && git read-tree --prefix=apps/apple/ "$GIT_COMMIT"' -- t-rel

# 3. Check the anchor before merging: the last release merged must rewrite to a commit of main.
#    For v1.1.6 that is 41d9d469.
git branch -f t-prev origin/release/v1.1.6
FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch -f --index-filter \
  'git read-tree --empty && git read-tree --prefix=apps/apple/ "$GIT_COMMIT"' -- t-prev
git -C <worktree> fetch "$PWD" t-rel:refs/upstream/rel t-prev:refs/upstream/prev
git -C <worktree> merge-base --is-ancestor refs/upstream/prev HEAD   # must succeed

# 4. Merge on the WP branch. A true merge: never squash or rebase it, or the next
#    release loses its base.
git -C <worktree> merge --no-ff refs/upstream/rel -m "Merge upstream DSPi-Console vX.Y.Z (WP SW-UPn)"
```

The fetched refs under `refs/upstream/` are scratch: delete them after the merge
(`git update-ref -d`). Only the merge commit carries the upstream commits into `main`.

**Resolving conflicts.** Classify every conflict before resolving it:

| Conflict | Resolution |
|---|---|
| Upstream against upstream (its own branches diverged; see section 4) | As upstream resolved it in its later release, if it has; otherwise take upstream's newest side |
| Upstream against the fork layer | Keep both; the fork layer's change is the one that moves |
| Upstream edits a command function that this repo has removed (modify/delete) | Keep the removal. Port the wire change to the core instead: a CG WP for `dspi-proto`/`dspi-core`, then the view call site. List it as a follow-up |
| Upstream edits a view that has an SW call site | Take upstream's view, re-apply the marked call-site edit |
| Upstream edits `project.pbxproj` | Take both sides' file entries; keep this repo's `Core/` package references |

**Follow-up list.** For every upstream commit that adds or changes a vendor request, a notification, a
wire struct or a preset field, the report names it, with the core WP that must carry it to the other
clients (Qt, Terminal, Android). Upstream's Swift is the reference for those features (PLAN-CLIENTS
section 1), so a release can create work in every client, not only in `apps/apple`.

## 4. Dry run (SW-U1)

Upstream has no release newer than `release/v1.1.6`, which `main` already contains: every upstream
`main` and `release/*` commit is in this history. Two upstream branches have commits that are not,
and both were dry-run merged into `main` at `e6db6e70` in a scratch clone
(`~/.cache/dspi-build/work/wp-SWU1/mono`), with the procedure of section 3:

| Upstream branch | Tip | Rewritten | Base found by `git merge-base` | New commits | Result |
|---|---|---|---|---|---|
| `overclock` | `69f90fb` 2026-08-04 "Added 1.5v" | `8e16460f` | `52f4809f` (upstream `0e179d7`, 2026-08-03) | 1 | Conflicts: `Commands.swift` 1 hunk, `DSPViewModel.swift` 1, `DSPi_ConsoleApp.swift` 8. 3 other files, +498, merged without conflict |
| `room-correction` | `fc565ea` 2026-07-30 | `d13c979d` | `76f0e938` (2026-07-26) | 57 | Conflicts: `project.pbxproj` 2, `Commands.swift` 1, `DSPi_ConsoleApp.swift` 2, `PresetSnapshot.swift` 1, `USBDevice.swift` 1. 109 other files, +29 899, merged without conflict |

**Findings.**

- **The base is found.** Both merges start from a real common ancestor, so the deterministic rewrite
  works for branches that left upstream before the import, not only for released tips.
- **No conflict comes from the core merge.** The same two merges into pure upstream `release/v1.1.6`
  (rewritten, `41d9d469`) give the same conflicting files with the same hunk counts. For
  `room-correction` the conflict hunks are identical line for line, only their line offsets differ. For
  `overclock` they are identical except one `DSPi_ConsoleApp.swift` hunk, which also carries the fork
  layer's iOS `applicationPages` block (8 more lines of context). The conflicts are upstream's
  own: both branches predate upstream's settings redesign and later release work.
- **Every conflict is outside `Core/`.** All are in files that points 2 and 3 of the rule will change
  (`Commands.swift`, `USBDevice.swift`, `DSPi_ConsoleApp.swift`) or in upstream-only files. Once
  `Commands.swift` is removed, its hunks become modify/delete conflicts, and the third row of the
  table in section 3 applies.
- **Follow-up a real merge would create.** `overclock` adds vendor requests `REQ_SET_SYS_CLOCK` (`0x40`)
  and `REQ_GET_SYS_CLOCK` (`0x41`) in `Commands.swift` and `Constants.swift`. `dspi-proto` and
  `dspi-core` have neither (the core's core-voltage support is the bofr firmware's extension channel,
  `vreg.rs`), and the firmware defines them only on its own `overclock` branch (`config.h:246`), not on `main` (`ec26bf0`). A merge would need a core WP for
  the two requests, then the Settings page call site.

Nothing was committed from the dry run. The scratch clones are in `~/.cache/dspi-build/work/wp-SWU1/`.

## 5. Things that break tracking

- Rewriting or rebasing anything under `apps/apple` that came from upstream (the anchors are those
  hashes).
- Squash-merging an upstream merge, or a review branch that contains one.
- A different rewrite (`git filter-repo`, `--subdirectory-filter` in reverse, or a prefix without the
  trailing slash): the hashes change and `git merge` finds no base.
- Moving `apps/apple` or renaming `DSPi Console/`. Upstream changes would then land in the old path.
  If a move is ever needed, it is one commit, and every later merge needs `-X find-renames`.
