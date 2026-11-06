# 原生桌面 GUI 原型

[English](README.md) | 中文

本项目包含 DeepSeek Harness 原生桌面界面的独立交互式设计原型。它用于展示视觉与交互行为，不是已发布的应用，也不是 Host 客户端。

## 从这里开始

- [`docs/architecture.md`](docs/architecture.md) 说明项目结构、状态归属、运行流程、响应式行为和限制。
- [`docs/interactions.md`](docs/interactions.md) 列出各界面、交互及模拟结果。
- [`index.html`](index.html) 是完整的原型入口文件，包含全部标记、样式、脚本和演示数据。

## 预览

在浏览器中直接打开 `index.html`。该文件无需安装依赖或执行构建，也不会连接 Desktop Host。

## 与应用的关系

已发布的应用位于 [`../src/`](../src/)，使用 Rust + gpui（`../src/gpui_main.rs`、`../src/shell/`）；`../src/main.rs` 中的旧 egui shell 已退役，仅在 `legacy-egui` feature 下构建。本原型不会包含在 Cargo 构建或 AppImage 中。原型专属文档保存在本目录；仓库级文档描述已发布产品和 Desktop lane。

gpui 迁移的冻结视觉与行为目标是 [`../DSH_UI_Handoff_v1.0/`](../DSH_UI_Handoff_v1.0/)，其中包含已批准的 HTML 参考、功能规格、runtime capability map 和实现 backlog。本原型仍是探索交互想法的场所；真正约束实现的是 handoff 目录。
