# `dspi hub` as a background service

Placeholders to adapt, one per OS (WP TM11). Each runs `dspi hub`, which shares
this machine's DSPi devices on the network (DSPi Link) until it is stopped. Run
`dspi hub --help` for its options.

| OS | File | Runs | Install |
|---|---|---|---|
| Linux | `dspi-hub.service` | systemd user service, while logged in (or always, with linger) | header of the file |
| macOS | `se.bofr.dspi.hub.plist` | launchd agent, while logged in | header of the file |
| Windows | `dspi-hub-task.xml` | scheduled task at logon | header of the file |

Each assumes `dspi` in a fixed place (`/usr/local/bin/dspi`,
`C:\Program Files\DSPi\dspi.exe`): edit the path to where it is installed.

## Pairing stays PIN-based

None of them passes `--open`, and none should: with open access anyone who can
reach the machine controls every device. With PIN pairing a client needs a PIN
that the hub shows only on its own terminal and only for two minutes. A PIN
crosses the network in plain text, so pair on a network you trust.

A service has no terminal, so pair with the service stopped:

1. Stop the service (`systemctl --user stop dspi-hub`, `launchctl bootout
   gui/$(id -u)/se.bofr.dspi.hub`, or end the task).
2. Run `dspi hub` in a terminal and type `pair`. It shows a PIN.
3. Connect from the client and enter the PIN. Type `quit`.
4. Start the service again. The client's token is kept in the hub's store and
   works across restarts.

`dspi hub --pair` opens a window as the hub starts and prints the PIN to its
output. Under a service that output is the log (journal, log file), so the PIN
would end up there. It stops working after two minutes or once used, but a log
is not a screen: prefer the steps above.

With the service stopped, `dspi hub clients`, `dspi hub role <cid> <role>` and
`dspi hub revoke <cid>` change the store directly. While a hub runs they refuse,
since the hub owns the store. On a terminal, the running hub takes the same
commands typed at it, and they take effect on live connections.

Switching between PIN and open access forgets every paired client. `dspi hub`
refuses to do that unless it is also given `--forget-clients`.

## Store, logs, sleep

- **Store:** the hub's id, paired clients (token hashes only) and device names,
  in the user's local data directory: `~/.local/share/dspi/hub` (Linux),
  `~/Library/Application Support/dspi/hub` (macOS), `%LOCALAPPDATA%\dspi\hub`
  (Windows). `--store DIR` to choose another. One hub per store.
- **Logs:** standard error, at `RUST_LOG` (warnings by default). Even at
  `trace`, the WebSocket library's frame dumps are held back, since they would
  carry PINs and tokens.
- **Sleep:** while it runs, `dspi hub` keeps the machine from sleeping. Linux:
  `systemd-inhibit` (a logind lock, gone when the hub ends). macOS: `caffeinate
  -i -w <pid>`. Windows: `SetThreadExecutionState`. If that is refused, it warns
  and shares anyway. `--no-keep-awake` turns it off.
- **Restarts:** a refused start (exit 1: another hub holds the store, or a change
  of access mode without `--forget-clients`; exit 4: the port is in use) is not
  retried: the unit has `RestartPreventExitStatus=1 4`, the launchd agent does not
  restart at all (launchd cannot tell a crash from a refusal), and the task tries
  at most 3 times. The log says why.
- **Stopping:** SIGTERM or Ctrl-C stops it cleanly. Clients are told and USB is
  released. A task ended from Task Scheduler is killed instead. That is safe, as
  the store is written atomically and the OS frees USB, but clients only see the
  connection drop.

## Windows: a service instead of a task

A scheduled task runs in the user's session with a console window. To run the
hub with no window and before anyone logs on, wrap it as a Windows service with
a service wrapper such as WinSW or NSSM, which runs `dspi.exe hub` and stops it
with Ctrl-C. Two things change:

- The service account's own data directory holds the store. Give it
  `--store C:\ProgramData\DSPi\hub` and make that directory readable only by the
  service account and administrators.
- A service needs permission to open the USB devices. WinUSB devices are
  usually open to the local system account.

`dspi` is not a native service (it does not answer the service control manager
itself), hence the wrapper.

## Firewall and network

The hub listens on TCP port 11915 (`--port`), and on mDNS so that clients find
it. Allow both on a private network only. Clients on the network cannot raise a
unit's core voltage above 1.20 V, whatever their role: do that on the machine
itself.
