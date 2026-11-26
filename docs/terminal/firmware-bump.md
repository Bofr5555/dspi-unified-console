# Moving to a newer firmware

The point of this procedure is that **no step involves changing existing code**.
The generator and the tests tell you exactly what is new; you add rows, not
edits. If you find yourself rewriting something, that is a signal the
architecture has sprung a leak, and it is worth fixing there instead.

Two vendored sets, bumped separately (`crates/dspi-proto/firmware/PROVENANCE.toml`):

- **The legacy headers** (steps 1 to 7): the 1.1.6-generation protocol, every
  `REQ_*` opcode, the bulk wire format and the constants around them.
- **The paged protocol** (step 8): upstream's `dspi_commands.json` and
  `legacy_map.json` for firmware 1.1.7 and later (PLAN-PG, OD-PGj).

**The legacy headers stay on the 1.1.6 line.** Once `bofr-main` has merged
upstream 1.1.7 (FWPG1), its `config.h` has no `REQ_*` defines any more, yet
`build.rs` generates every legacy opcode (`generated::opcodes`, `ALL_OPCODES`)
from them, and the planner's coverage test holds `legacy_map.json` equal to
them. So from then on the headers are pinned at the tag `bofr-116-final` (the
last 1.1.6-generation commit of `bofr-main`, which contains PAGEGUARD) and are
not re-copied from a 1.1.7-generation commit. A change to the bulk wire format
on the 1.1.7 line is copied header by header with that in mind (only
`bulk_params.h` and the headers it needs), never as the whole set.

## 1. Copy the headers

The vendored set is the `files` list in `crates/dspi-proto/firmware/PROVENANCE.toml`,
which is also the `HEADERS` list in `crates/dspi-proto/build.rs`. At `557bce7` it
is thirteen headers:

```sh
FW=/path/to/DSPi                     # on the release branch you are targeting
DST=crates/dspi-proto/firmware

for h in config.h bulk_params.h control_surfaces.h psybass.h upmix.h siggen.h \
         subharm.h tube.h limiter.h rta.h rta_fft.h rta_bass.h notify.h; do
  cp "$FW/firmware/DSPi/$h" "$DST/$h"
done

( cd "$FW" && git rev-parse HEAD && git rev-parse --abbrev-ref HEAD \
           && git log -1 --format=%ci HEAD )
```

A new firmware feature usually brings a new header. Add it to `HEADERS` in
`build.rs` and to the `files` list in `PROVENANCE.toml` together; a header that
is copied but not listed is never read.

Update `PROVENANCE.toml` with the new branch, commit, commit date and the date
you vendored, then refresh the checksums:

```sh
( cd crates/dspi-proto/firmware && shasum -a 256 *.h > SHA256SUMS )
```

Track the **release branch**, not `main`.

## 2. Rebuild

```sh
cargo build -p dspi-proto
```

The generator lifts three kinds of constant, each by name prefix:

| List in `build.rs` | Lifts | Lands in |
|---|---|---|
| `DEFINE_GROUPS` | integer `#define`s such as `REQ_*`, `CS_*`, `TUBE_*`, `RTA_*` | `generated::<group>`, e.g. `generated::tube` |
| `ENUM_PREFIXES` | `enum` members, explicit or implicit, such as `TUBE_PARAM_*`, `LIMITER_PARAM_*`, `PARAM_SRC_*`, `CS_TYPE_*`, `CS_NOUN_*` | the group whose prefix they match |
| `FLOAT_PREFIXES` | float `#define`s such as `SUBHARM_LEVEL_MAX` | `generated::ranges` |

A new header's constants appear only once their prefix is listed. Anything
parenthesised or computed is skipped rather than evaluated.

Watch the build log. Any constant that became platform-conditional is reported
as `platform-conditional, not emitted: NAME`. That is the generator refusing to
let a per-platform value become a compiled-in assumption; the value must come
from a capability probe instead. (`SIGGEN_MULTITONE_MAX` has been reported since
v1.1.6 and is read from the signal generator's caps.)

## 3. Let the tests tell you what moved

```sh
cargo test --workspace
```

Expect these to speak up first:

| Failing test | Means |
|---|---|
| `bulk_size_matches_firmware` | The packet changed size. Find the section that grew or was appended, fix `sections` in `build.rs`, and re-derive `docs/wire-format.md`. |
| `no_v28_section_moved` | A section moved rather than being appended. That is a model change; see step 5. |
| `sections_are_contiguous_and_complete` | The section list in `build.rs` no longer matches the packet. Add or resize the entry. |
| `every_opcode_was_generated` | Opcodes were added or removed. |
| `wire_format_version_is_known` | The wire version bumped. Read the `WIRE_FORMAT_VERSION` comment in `bulk_params.h`, which documents every version's change in one line. |
| `refuses_an_unknown_version_rather_than_guessing` | Update the expected version once you have implemented the new layout. |
| `the_noun_and_type_tables_match_the_vendored_header` | The control-surface nouns or component types changed. Extend `CS_NOUNS` / `CS_TYPES` in `packets.rs` and `CsNoun` / `CsType` in `enums.rs`. |
| `the_expected_firmware_is_the_vendored_one` | `FW_VERSION_*` moved, which is also the version a refused device is told to update to. |

Tests that pin a count or a version (`every_opcode_was_generated`,
`wire_format_version_is_known`, `bulk_size_matches_firmware`) are updated by hand
with a comment citing the header line, which is the point: someone has to read
what changed.

## 4. Handle each new opcode

For every opcode the coverage test reports, either:

- add a registry row (the normal case), or
- add it to the exclusion list **with a written reason**, for opcodes that are
  genuinely not host-driveable.

Never silence the test by loosening the assertion.

## 5. Handle each wire change

Read the new `WIRE_FORMAT_VERSION` comment carefully. Changes fall into classes,
and each has a standard response:

There is no field-descriptor table. What exists is:

- **`sections` in `emit_wire_layout` (`build.rs`)**, the one hand-sized list:
  `(name, size)` in declaration order, sizes computed from the `WIRE_MAX_*`
  constants where the header does. It generates `OFF_<SECTION>`,
  `LEN_<SECTION>`, `SECTIONS` and `BULK_SIZE`.
- **Field offset tables in `crates/dspi-proto/src/wire.rs`**, one per section
  whose fields someone has had to pin: `INPUT_CONFIG_FIELDS`, `SUBHARM_FIELDS`,
  `TUBE_FIELDS` and `LIMITER_OUTPUT_FIELDS`, each `(name, offset, length)` with a
  test that the fields are contiguous and at the header's offsets.
- **Section decoders in `crates/dspi-session/src/state.rs`** (`decode_global`,
  the `Psybass` and `Upmix` views and so on), which read fields from a section
  slice at fixed offsets and are what the screens use.

| Class | Response |
|---|---|
| Section appended | Add an entry to `sections` in `build.rs`, a field table in `wire.rs` with its offset test, and a decoder in `state.rs`. |
| Struct grew | Change its size in `build.rs`; offsets after it shift automatically. Extend the field table. |
| Reserved byte claimed | Add the field to the section's table and decoder. **Check whether it uses a `+1` sentinel**, which several do. |
| New enum value | Add a variant. The `Unknown` fallback means old builds already tolerated it. |
| Model change | The serious case. Update `ChannelMap` and the affected rows. |

**Sentinel encodings are the trap.** Fields added by claiming reserved bytes use
`0 = absent, keep the live value`, and where zero is meaningful they are stored
plus one. Writing a plain zero disables things silently. See
`docs/wire-format.md` section 5, and use `wire::decode_p1` / `wire::encode_p1`.

## 6. Verify against hardware

```sh
cargo run --bin uconsole -- dump
```

Confirm the channel count, wire version, firmware version (with its beta),
build and feature list match the device you are holding. A device on another
wire version is refused with a message naming its firmware; that message is the
expected result for an older unit. On macOS, quit DSPi Console first: the vendor
interface is exclusive.

## 7. Write down anything surprising

Record it in `docs/firmware-notes.md`, **in this repository**. The firmware repo
is not modified as part of this work.

## 8. The paged protocol (`dspi_commands.json`, `legacy_map.json`)

`dspi_proto::paged` is generated from two files of upstream's firmware repository,
`Documentation/protocol/dspi_commands.json` (the command registry: ids,
directions, address kinds, GET lengths, SET bounds, flags, build conditions,
page revisions) and `Documentation/protocol/legacy_map.json` (every 1.1.6 opcode
and how it is reproduced on paged commands). Both come from **one** commit of the
release branch, from the firmware repository itself (GPL-3.0 project files),
never from DSPi-Link's copy. They are data: nothing is transcribed from them by
hand except the 14 `custom` rows (`paged/custom.rs`, from the migration guide).

```sh
FW=/path/to/DSPi                     # upstream, on the release branch you are targeting
REV=<commit>
DST=crates/dspi-proto/firmware

for f in dspi_commands.json legacy_map.json; do
  git -C "$FW" show "$REV:Documentation/protocol/$f" > "$DST/protocol/$f"
done
git -C "$FW" log -1 --format='%H %ci' "$REV"
```

Then, in `PROVENANCE.toml`'s `[protocol]` section:

1. Set `branch`, `commit`, `short`, `committed` and `vendored`.
2. Run the coverage test **before** touching a count:
   `cargo test -p dspi-proto --test paged_coverage`. It compares what `build.rs`
   read with the recorded counts (pages, commands per page, GET and SET commands,
   protocol major and minor, opcodes, adapters per USB direction and per kind,
   split cases) and fails on any row the generator could not read
   (`legacy::UNREAD`; the build log names each one as `paged: not generated`).
3. Read what moved (upstream's diff of the two files and of
   `Documentation/Features/vendor_protocol_migration_guide.md`), then re-record
   the counts. A new adapter kind, address kind, flag, status family or result
   name is an unread row: teach `build.rs` (its `paged` module) and the types in
   `src/paged/legacy.rs` or `registry.rs` about it first.
4. Refresh the checksums: `( cd crates/dspi-proto/firmware && sha256sum *.h protocol/*.json > SHA256SUMS )`.

Expect these to speak up as well:

| Failing test | Means |
|---|---|
| `wp_pgproto_every_header_opcode_has_a_row` | `legacy_map.json` and the pinned legacy headers disagree on an opcode. Upstream's map is the 1.1.6 opcode set; our headers add only `REQ_EXTENSION`. |
| `wp_pgproto_seam_literals_are_the_generated_ones` | The probe (`SYS.PROTOCOL_INFO` 0x0110, 16 bytes) or the protocol major moved: that is a new protocol generation, not a bump. |
| `paged_guide_vectors` | A byte example transcribed from the migration guide no longer holds. Re-read the guide's section and fix the vector or the generator, never both at once. |
| `proptest_wp_pgproto_planner_then_reverse`, `wp_pgproto_every_simple_row_plans_and_reverses` | A new row is not invertible by the reverse view; look at how its address or payload is built. |

A device on a **higher page revision** than the vendored one still works: the
planner then skips its local SET-length checks for that page and lets the device
judge (PLAN-PG R8b). A bump is needed for new commands, not for the device to keep
working.

Re-vendoring at upstream's release tag, and our fork's regenerated
`dspi_commands.json` with the extension page 0xE0 row, is PLAN-PG's PGREV.
