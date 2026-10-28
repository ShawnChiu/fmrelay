# fmrelay

`org.freedesktop.FileManager1` for any file manager: route "Show in folder" to a command.

Browsers ("Show in folder" in the downloads list), Electron apps and the desktop portal ask
for a file manager over D-Bus, through `org.freedesktop.FileManager1`. On a window manager
without a desktop environment, either nobody answers that call, or whatever file manager got
pulled in as a dependency does (often Nautilus, through xdg-desktop-portal-gnome). fmrelay
answers it instead and runs the command you give it, so the request can open yazi in a
terminal, or any other file manager.

```sh
fmrelay --folders 'ghostty -e yazi {}'
```

## How it works

fmrelay is a small daemon on the session bus. It serves the three methods of the
[File Manager D-Bus interface](https://www.freedesktop.org/wiki/Specifications/file-manager-interface/):

| Method               | Command used                           |
| -------------------- | -------------------------------------- |
| `ShowFolders`        | `--folders`                            |
| `ShowItems`          | `--items` (default: `--folders`)       |
| `ShowItemProperties` | `--properties` (default: `--items`)    |

Each call returns right away; the command runs in the background. Only `file://` URIs are
handled; anything else is logged and skipped.

The caller's startup ID is passed on as `XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID`, so
the compositor can give focus to the new window.

## Requirements

- Linux with a systemd user session, and a compositor or window manager that starts
  `graphical-session.target` (so programs started by systemd can open windows).
- A D-Bus session bus: dbus-broker (tested) or dbus-daemon. Both let service files in
  `~/.local/share/dbus-1/services/` override the ones in `/usr/share/dbus-1/services/`.
- A recent stable Rust to build (the locked dependencies need 1.89 or newer).

## Install

```sh
cargo install --locked --git https://github.com/ShawnChiu/fmrelay
```

Then install the systemd unit and the D-Bus activation file from `contrib/`:

```sh
install -Dm644 contrib/fmrelay.service ~/.config/systemd/user/fmrelay.service
install -Dm644 contrib/org.freedesktop.FileManager1.service \
    ~/.local/share/dbus-1/services/org.freedesktop.FileManager1.service
```

Edit `ExecStart=` in `~/.config/systemd/user/fmrelay.service` to your file manager (see the
examples below), then:

```sh
systemctl --user daemon-reload
systemctl --user enable --now fmrelay.service
```

The activation file makes the bus start `fmrelay.service` whenever something calls
`org.freedesktop.FileManager1` and fmrelay isn't running, instead of starting the system file
manager.

Check that fmrelay owns the name:

```sh
busctl --user status org.freedesktop.FileManager1 | grep -E '^(PID|Comm)='
```

If it shows another file manager (Nautilus, for example), that one got the name first and
doesn't let it go: fmrelay waits in the bus queue and takes over when it exits.

## Command templates

Each command is split like a shell command line (no shell is run) and must contain a
placeholder; fmrelay refuses to start otherwise.

- `{}` is the path itself.
- `{parent}` is the folder that holds it, for file managers that can't select a file.
  Several items in the same folder open that folder once.

How a placeholder is expanded depends on where it is:

- An argument that is **exactly** `{}` or `{parent}` becomes every path, and the command
  runs once: `nautilus --select {}` → `nautilus --select /a/x /b/y`.
- A placeholder **inside** a longer argument holds one path, so the command runs once per
  path: `fm --select={}` → `fm --select=/a/x`, then `fm --select=/b/y`.

## Examples

Both were tested through fmrelay with `ShowFolders` and `ShowItems`. `ghostty -e` runs
yazi in a terminal; use your terminal's equivalent.

| File manager | Options                                                  | `ShowItems`                                             |
| ------------ | -------------------------------------------------------- | ------------------------------------------------------- |
| yazi         | `--folders 'ghostty -e yazi {}'`                         | cursor on the item; several items open as tabs          |
| Nautilus     | `--folders 'nautilus {}' --items 'nautilus --select {}'` | item selected; several items open as tabs in one window |

In the systemd unit, `ExecStart=` takes the same quoting:

```ini
ExecStart=%h/.cargo/bin/fmrelay --folders 'nautilus {}' --items 'nautilus --select {}'
```

## Uninstall

```sh
systemctl --user disable --now fmrelay.service
rm ~/.config/systemd/user/fmrelay.service \
    ~/.local/share/dbus-1/services/org.freedesktop.FileManager1.service
systemctl --user daemon-reload
cargo uninstall fmrelay
```

The system file manager answers `org.freedesktop.FileManager1` again the next time it is
started.

## Development

Build:

```sh
cargo build
```

Check before every commit (each commit must pass on its own):

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Integration test on a private session bus (needs `dbus-run-session` and `busctl`; never
touches the real session bus or opens a window):

```sh
scripts/test-bus.sh
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
