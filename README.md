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
cargo fmt --check && cargo clippy -- -D warnings && cargo build
```
