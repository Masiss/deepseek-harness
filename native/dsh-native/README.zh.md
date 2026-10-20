# dsh-native

[English](README.md) | 中文

DeepSeek Harness Desktop 运行时的原生提供方。

## 职责

一个 napi-rs 模块替换 Desktop payload 原本随包分发的原生 addon。第一个提供方是终端：`node-pty` 由 `portable-pty` 实现替换，并通过同一套 JavaScript 接口暴露，因此 `packages/subprocess/subprocess-local` 保留它已经声明的提供方。

| 提供方 | 替换 | 接口 |
|---|---|---|
| PTY | `node-pty` | `spawn`、`pid`、`write`、`resize`、`pause`、`resume`、`kill`、`onData`、`onExit` |

退出报告带出子进程死于哪个信号编号；指定信号时，信号发给终端的进程组。

## 运行时如何用到它

`apps/desktop-host/native/pty-loader.mjs` 注册一个 resolve hook，把 `node-pty` 指向 `apps/desktop-host/native/node-pty.cjs`——调用本模块的包装层。Desktop launcher 在 profile 启动前用 `--import` 加载该 hook；Electron 载体继续使用 node-pty。

没有编译进本模块的 payload 继续使用 node-pty：除非包装层和模块同时存在，否则 hook 不做任何事；Windows 继续使用 node-pty 的 ConPTY 宿主。

## 构建

```sh
apps/desktop-tauri/scripts/build-native-pty.sh
```

脚本执行 `cargo build --release` 并把产物复制到 `apps/desktop-host/native/dsh-native.node`。Desktop 打包步骤（`apps/desktop-tauri/scripts/pack-linux.sh`）在打包 Host package 之前运行它，因此模块随 payload 一起分发。

## 验证

```sh
apps/desktop-tauri/scripts/native-pty-probe.sh
```

probe 在本提供方上驱动已打包运行时的终端服务，并报告解析到的提供方、pty 窗口尺寸、回显的输入和终止信号。
