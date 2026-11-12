# Releasing

## What CI proves

`.github/workflows/ci.yml` runs on every push. It does three things the local
loop cannot:

- **Builds on all three desktop platforms**, not just the developer's. `cargo
  check` proves code compiles; only a real build proves it links, and until CI
  ran, nothing but macOS had ever been linked.
- **Cross-compiles both Raspberry Pi targets** with `cross`, which links against
  a real ARM sysroot rather than type-checking against a target spec.
- **Parses the generated shell completions** with `bash -n`, `zsh -n` and
  `fish --no-execute`. A completion script that fails to parse is worse than
  none, because it breaks the user's shell startup.

Clippy runs with `-D warnings`, so a lint is a build failure rather than
something to scroll past.

## What CI still does not prove

**No test touches hardware.** Every USB path is exercised against
`MockTransport`, which models stalls, timeouts and the flash blackout, but a
mock cannot be wrong in the same way a device can. The hardware smoke suite
(`--features hardware`, still to be written) is the missing piece, and it needs
a device attached to a runner.

Until then, `uconsole doctor` on the target machine is the real check. It
enumerates, opens, claims the interface and probes the firmware, which is
exactly the sequence that fails when a platform assumption is wrong.

## Cutting a release

```sh
git tag v2.0.0
git push origin v2.0.0
```

`release.yml` builds seven targets and uploads an archive for each, carrying the
binary, the README and the udev rule. The rule ships in every archive, not only
the Linux ones: it costs nothing and means the fix is always to hand when
someone unpacks the wrong archive on a Pi.

## Version numbers

The application version is independent of the firmware version. `uconsole --version`
prints both, plus the wire format:

```
uconsole 2.0.0 (protocol from DSPi release/v1.1.5 @ 9776c2f)
wire format V26, 190 vendor opcodes
```

That pairing is what makes a bug report actionable: "wire V26" says more about
what the app was talking to than an app version ever could.

## Release archives and the source archive (WP CLEAN1)

Every release publishes, beside its downloads, the source archive of the exact commit it
was built from: `ci/release-source.sh VERSION dist` writes
`dspi-uconsole-VERSION-source.tar.gz` (a `git archive`, without `apps/desktop` but its shared
`packaging/licences/` while the Qt app is not released; `--with-desktop` for a Qt release). The notices of every non-Qt artifact point at that file.

A Terminal archive comes from a release build of `-p dspi-term`:
`ci/release-terminal.sh target/release/uconsole linux-x86_64 VERSION dist`. It holds the
binary under its command name (`ci/command-name.sh`), the README, `LICENSE`, `NOTICE.md`,
`THIRD-PARTY-NOTICES.txt` (the Rust crates' licence texts,
`apps/terminal/notices/rust-crates.txt`), `SOURCE.txt`, the udev rule and the hub's
service files (`service/`, rendered from `apps/terminal/packaging/service/`). Regenerate the
crates' texts after a dependency change with
`ci/run.sh 'python3 apps/terminal/tools/gen_rust_notices.py'`; the `terminal` check step
fails while they are stale.
