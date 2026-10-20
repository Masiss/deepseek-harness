# dsh-native

English | [中文](README.zh.md)

Native providers for the DeepSeek Harness Desktop runtime.

## Scope

One napi-rs module replaces the native addons the Desktop payload used to
ship, so the Linux payload carries no FFI library, no C++ addon, and no C
addon. Each provider exposes the surface the Harness already declares, so
`packages/` keeps its existing providers.

| Provider | Replaces | Surface |
|---|---|---|
| PTY | `node-pty` | `spawn`, `pid`, `write`, `resize`, `pause`, `resume`, `kill`, `onData`, `onExit` |
| Session lease | `@deepseek-ai/node-addon-system/flock` | `tryLockExclusive` |
| Process bootstrap | `koffi` | `execve`, `fcntl` |

The PTY exit report carries the signal number the child died from, and the
terminal's process group receives the signal when one is named. The lease
takes an exclusive, nonblocking lock on a descriptor the caller owns and
leaves it in place until the descriptor closes. The bootstrap replaces the
process image with the given program, argv, and environment, and reports the
errno of a refused exec.

## How the runtime reaches it

`apps/desktop-host/native/native-loader.mjs` registers a resolve hook that
points `node-pty` at `apps/desktop-host/native/node-pty.cjs`, the session
lease's `@deepseek-ai/node-addon-system/flock` entry at
`apps/desktop-host/native/flock.cjs`, and `koffi` at
`apps/desktop-host/native/koffi.cjs`. The Desktop launcher loads that hook
with `--import` before the profile boots; the Electron carrier keeps the
bundled addons.

A payload without the built module keeps the bundled implementations: the hook
does nothing unless the module is present, and the payload keeps the addons
the module would replace. Windows keeps node-pty's ConPTY host and the
Windows-only koffi callers.

The `koffi` shim serves the two libc signatures the Harness binds and refuses
any other one, so a new binding fails loudly instead of silently doing
nothing.

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
apps/desktop-tauri/scripts/native-flock-probe.sh
```

The first probe drives the packaged runtime's terminal service on this
provider and reports the resolved provider, the pty window size, the echoed
input, and the termination signal. The second resolves the lease and libc
entries through the same hook and reports the lock contention, the errno, and
the replaced process image.
