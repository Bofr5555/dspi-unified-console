# Changelog

Release notes of earlier versions: `CHANGELOG-0.13.0.md`.

## Unreleased

### Terminal
- The command is now `uconsole`. `dspi` and `dspi-term` are no longer built or installed,
  and nothing is kept for the old name: scripts, shell completions and service files that
  call `dspi` need the new name. Regenerate completions with `uconsole completions <shell>`.
- Its files moved with it: settings and Link pairing tokens to `~/.config/uconsole/` (the
  platform's configuration directory), the AutoEQ database to the same directory, the
  hub's store to `~/.local/share/uconsole/hub` (the platform's local data directory), and
  a web console installed beside it to `share/uconsole/web/`. Nothing is moved over from
  the old directories.
- The hub's service files are named after the command (`uconsole-hub.service`,
  `se.bofr.uconsole.hub.plist`, `uconsole-hub-task.xml`).
- `uconsole --third-party-licences` prints the licence texts of the Rust crates compiled
  into it; the release archives carry them as `THIRD-PARTY-NOTICES.txt`.

### All clients (draft notices)
- About no longer shows the first-person donation paragraph or "Made with love by Weeb
  Labs"; the credit line reads "DSPi and its firmware by Weeb Labs", and the links section
  says where to support Weeb Labs' work.
- The licence notice says that the program as a whole is distributed under the GNU GPL
  version 3, with Bofr5555's own contributions under version 3 or later and the section 7
  terms; it names the source archive published with each release instead of a repository
  link, and credits the firmware's third-party parts, Noto Sans Mono and AutoEq.
