# dsh-native

English | [中文](README.zh.md)

Native providers for the DeepSeek Harness Desktop runtime.

## Scope

One napi-rs module replaces the native addons the Desktop payload used to
ship. The first provider is the terminal: `node-pty` is replaced by a
`portable-pty` implementation exposed through the same JavaScript surface, so
`packages/subprocess/subprocess-local` keeps the provider it already declares.

| Provider | Replaces | Surface |
|---|---|---|
| PTY | `node-pty` | `spawn`, `pid`, `write`, `resize`, `pause`, `resume`, `kill`, `onData`, `onExit` |

The exit report carries the signal number the child died from, and the
terminal's process group receives the signal when one is named.

## How the runtime reaches it

`apps/desktop-host/native/pty-loader.mjs` registers a resolve hook that points
`node-pty` at `apps/desktop-host/native/node-pty.cjs`, the wrapper that calls
this module. The Desktop launcher loads that hook with `--import` before the
profile boots; the Electron carrier keeps node-pty.

A payload without the built module keeps node-pty: the hook does nothing unless
both the wrapper and the module are present, and Windows keeps node-pty's
ConPTY host.

## Build

```sh
apps/desktop-tauri/scripts/build-native-pty.sh
```

The script runs `cargo build --release` and copies the result to
`apps/desktop-host/native/dsh-native.node`. The Desktop packaging step
(`apps/desktop-tauri/scripts/pack-linux.sh`) runs it before the Host package is
packed, so the module ships inside the payload.

## Verify

```sh
apps/desktop-tauri/scripts/native-pty-probe.sh
```

The probe drives the packaged runtime's terminal service on this provider and
reports the resolved provider, the pty window size, the echoed input, and the
termination signal.
