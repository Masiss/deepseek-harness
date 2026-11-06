# Agent Note: 删除已退役的 egui 桌面 shell

Status: implemented

[English](2026-11-06-delete-retired-egui-desktop-shell.md) | 中文

## Problem

`apps/desktop-tauri/native-app` 曾同时产出一个 gpui 二进制和一个过渡用的 egui 二进制（`dsh-desktop-egui-legacy`），后者藏在 `legacy-egui` feature 之后。两者用同一套协议与偏好模块渲染相同的 V2 界面，因此每次 UI 改动都要做两遍。这种重复确实造成了漂移：先加到 gpui 的功能很晚才到 egui，措辞还不一致；没人发布的 egui 会先编译失败；只为 gpui 有意义的改动也要重录它的截图基线。

## Decision

原生应用只有**一个**二进制 `dsh-desktop-native-app`（`src/gpui_main.rs` 加 `src/shell/`）。egui shell、其 feature，以及一切只为构建或测试它而存在的东西都被删除：`src/main.rs`、`src/ui_harness.rs` 与 `src/ui_harness/audit.rs`、`src/settings_panel.rs`、`src/transcript_view.rs`、`tests/snapshots/*.png`、`legacy-egui` feature、可选依赖 `eframe`、开发依赖 `egui_kittest`，以及共享模块中所有 `#[cfg(feature = "legacy-egui")]` 块。`fonts.rs` 现在只导出 `contains_cjk`；字体目录与 egui 字体链辅助函数已移除，`document_preview`、`explorer`、`question_card`、`session_list`、`terminal_ansi`、`layout`、`ui_preferences` 中的 egui 专用辅助同样如此。打包旅程脚本里的 `dsh-desktop-egui-legacy` 目标与全部 `DSH_NATIVE_UI_*` 探针接线一并删除。

共享模块保持与渲染器无关。`KeyChord`、`Rgba8`、`AnsiColor`、`Transcript`、偏好与类型化客户端维持原有契约，gpui 仍是这个无框架内核的一个消费者。

## Alternatives considered

- **保留 egui 二进制可构建但冻结。** 否决：它仍需随每个共享模块改动而编译，恰恰是产生漂移的维护成本，而无人发布它。
- **维持两个 shell 完全对等。** 否决：让每个 UI 切片的成本翻倍，却服务一个没有 release 使用的渲染器。
- **删除二进制但保留 egui harness。** 否决：harness 驱动的是 egui 渲染器，无法脱离它存活；仅为测试保留 eframe 又会把移植已移除的依赖带回来。

## Consequences

只剩一个渲染器、一套界面，约一万二千行第二实现被移除。共享内核不再携带按 feature 门控的呈现代码。

代价是测试证据。`egui_kittest` harness 曾是本 lane 唯一的无窗口 UI harness：它提供 AccessKit 交互测试、截图基线，以及 Marketplace、Office 预览、PTY 面板、profile 默认模型与签名更新弹窗的 L2 打包旅程。它消失后，应用的自动化覆盖是 gpui 模块上的 `cargo test` 加上环境变量探针（`terminal`、`timed-question`、`model-default`），布局与视觉保真只能依靠人工审阅。在 gpui 上重建等价 harness 是未完成事项，在其存在之前不得声称打包 UI 旅程通过。因此 `scripts/native-journey-probe.sh` 只暴露 `terminal`、`timed-question`、`model-default`；`office-component-probe.sh`、`model-default-probe.sh`、`update-flow-probe.mjs` 保留 Host 层检查而不含 GUI 部分。
