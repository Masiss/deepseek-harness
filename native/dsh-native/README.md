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
| Image pipeline | `sharp` | decode, metadata, orientation, scale, WebP and JPEG encoding |

The PTY exit report carries the signal number the child died from, and the
terminal's process group receives the signal when one is named. The lease
takes an exclusive, nonblocking lock on a descriptor the caller owns and
leaves it in place until the descriptor closes. The bootstrap replaces the
process image with the given program, argv, and environment, and reports the
errno of a refused exec.

The image provider reports the same intrinsic facts as the package it
replaces — format, dimensions, frame count, sample depth, colour space, alpha,
and the metadata a container carries — and encodes WebP and JPEG at the same
quality target. Every other operation of the image package is delegated to it
when the payload still carries it.

The crate also builds the Landlock launcher the sandbox uses: the same
argument grammar, exit codes, report lines, and confinement effects as the C
launcher it replaces, with the Landlock UAPI defined in Rust.

## How the runtime reaches it

`apps/desktop-host/native/native-loader.mjs` registers a resolve hook that
points `node-pty` at `apps/desktop-host/native/node-pty.cjs`, the session
lease's `@deepseek-ai/node-addon-system/flock` entry at
`apps/desktop-host/native/flock.cjs`, `koffi` at
`apps/desktop-host/native/koffi.cjs`, and `sharp` at
`apps/desktop-host/native/sharp.cjs`. The Desktop launcher loads that hook
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
apps/desktop-tauri/scripts/build-native.sh
```

The script runs `cargo build --release` and copies the result to
`apps/desktop-host/native/dsh-native.node`, then builds the Landlock launcher
and copies it to the target tree, where the runtime preparation puts it in
place of the C launcher. The Desktop packaging step
(`apps/desktop-tauri/scripts/pack-linux.sh`) runs the script before the Host
package is packed, so the module ships inside the payload.

## Verify

```sh
apps/desktop-tauri/scripts/native-pty-probe.sh
apps/desktop-tauri/scripts/native-flock-probe.sh
apps/desktop-tauri/scripts/image-parity-probe.sh
apps/desktop-tauri/scripts/landlock-parity-probe.sh
apps/desktop-tauri/scripts/landlock-sandbox-probe.sh
```

The probes report the resolved provider for the terminal, the session lease,
and the image pipeline; the image probe compares the intrinsic facts and the
encoded sizes with the package it replaces; the Landlock probes compare the
launcher's whole observable behavior with the C launcher and confine real
commands through the sandbox provider.
