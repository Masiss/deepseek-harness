# dsh-native

[English](README.md) | 中文

DeepSeek Harness Desktop 运行时的原生提供方。

## 职责

一个 napi-rs 模块替换 Desktop payload 原本随包分发的原生 addon，因此 Linux payload 不再需要 FFI 库、C++ addon 和 C addon。每个提供方都暴露 Harness 已经声明的接口，因此 `packages/` 保留它现有的提供方。

| 提供方 | 替换 | 接口 |
|---|---|---|
| PTY | `node-pty` | `spawn`、`pid`、`write`、`resize`、`pause`、`resume`、`kill`、`onData`、`onExit` |
| Session lease | `@deepseek-ai/node-addon-system/flock` | `tryLockExclusive` |
| 进程 bootstrap | `koffi` | `execve`、`fcntl` |
| 图像流水线 | `sharp` | 解码、元数据、方向、缩放、WebP 与 JPEG 编码 |

PTY 的退出报告带出子进程死于哪个信号编号；指定信号时，信号发给终端的进程组。lease 对调用方拥有的描述符加排他、非阻塞锁，锁一直保持到描述符关闭。bootstrap 用给定的程序、argv 和环境替换进程映像，exec 被拒绝时报告 errno。

图像提供方报告与被替换包相同的固有事实——格式、尺寸、帧数、采样深度、色彩空间、alpha，以及容器携带的元数据——并以相同的质量目标编码 WebP 和 JPEG。图像包的其他操作在 payload 仍携带它时转交给它处理。

crate 同时构建 sandbox 使用的 Landlock 启动器：参数语法、退出码、报告行和约束效果都与它替换的 C 启动器一致，Landlock UAPI 在 Rust 中定义。

## 运行时如何用到它

`apps/desktop-host/native/native-loader.mjs` 注册一个 resolve hook，把 `node-pty` 指向 `apps/desktop-host/native/node-pty.cjs`，把 session lease 的 `@deepseek-ai/node-addon-system/flock` 入口指向 `apps/desktop-host/native/flock.cjs`，把 `koffi` 指向 `apps/desktop-host/native/koffi.cjs`，把 `sharp` 指向 `apps/desktop-host/native/sharp.cjs`。Desktop launcher 在 profile 启动前用 `--import` 加载该 hook；Electron 载体继续使用随包分发的 addon。

没有编译进本模块的 payload 继续使用随包分发的实现：除非模块存在，否则 hook 不做任何事，payload 也保留本模块会替换的那些 addon。Windows 继续使用 node-pty 的 ConPTY 宿主和仅限 Windows 的 koffi 调用方。

`koffi` 包装层只提供 Harness 绑定的那两个 libc 签名，其他签名一律拒绝，这样新增绑定会显式报错，而不是悄悄失效。

## 构建

```sh
apps/desktop-tauri/scripts/build-native.sh
```

脚本执行 `cargo build --release` 并把产物复制到 `apps/desktop-host/native/dsh-native.node`，随后构建 Landlock 启动器并复制到目标目录，运行时准备步骤用它替换 C 启动器。Desktop 打包步骤（`apps/desktop-tauri/scripts/pack-linux.sh`）在打包 Host package 之前运行该脚本，因此模块随 payload 一起分发。

## 验证

```sh
apps/desktop-tauri/scripts/native-pty-probe.sh
apps/desktop-tauri/scripts/native-flock-probe.sh
apps/desktop-tauri/scripts/image-parity-probe.sh
apps/desktop-tauri/scripts/landlock-parity-probe.sh
apps/desktop-tauri/scripts/landlock-sandbox-probe.sh
```

probe 报告终端、session lease 和图像流水线解析到的提供方；图像 probe 对比固有事实和编码体积与被替换包的结果；Landlock probe 对比启动器的全部可观察行为与 C 启动器，并通过 sandbox 提供方约束真实命令。
