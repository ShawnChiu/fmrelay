# fmrelay

`org.freedesktop.FileManager1` for any file manager: route "Show in folder" to a command.

Work in progress.

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
