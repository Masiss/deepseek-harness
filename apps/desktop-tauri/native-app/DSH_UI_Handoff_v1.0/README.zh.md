# DSH UI Handoff v1.0

[English](README.md) | 中文

原生 Desktop 界面的交接文档集：行为规范、运行时能力地图、实现 backlog，以及一份作为视觉基准的 HTML。

## 内容

| 文件 | 作用 | 权威范围 |
|---|---|---|
| [`DSH_UI_Functional_Spec_v1.0.md`](DSH_UI_Functional_Spec_v1.0.md) | 行为契约：功能 id、验收标准、R0–R5 路线图 | 预期*行为*的权威 |
| [`runtime-capability-map.md`](runtime-capability-map.md) | 把每条需求对应到已有的 Host RPC 与 gpui shell | *实现状态*的权威（文内注明审计日期） |
| [`DSH_UI_Implementation_Backlog_v1.0.md`](DSH_UI_Implementation_Backlog_v1.0.md) | 执行顺序、验收 gate、报告格式 | *工作顺序*的权威 |
| [`dsh-ui-prototype-v2.html`](dsh-ui-prototype-v2.html) | 自包含 HTML：布局、颜色 token、面板、状态 | *视觉*的权威 |

## 与 `native-app` 其余部分的关系

- `../src/` — 实际发布的应用：**Rust + gpui**（`src/gpui_main.rs`、`src/shell/`）。egui shell（`src/main.rs`）已于 2026-11-06 删除。
- `../prototype/` — 更早的交互原型（`index.html` + `docs/`），描述 egui 时期的界面；查看流程仍有用，但不是本次交接的视觉基准。
- `DSH_UI_Handoff_v1.0/` — gpui 移植的视觉与行为基准。
- `../USER_GUIDE.md` 与 `../USER_FLOWS.md` — 描述正在运行的应用；当本文档集与应用不一致时，应用是事实，本文档集是目标。

当规范中的某项行为没有对应 Host 能力时，记录 `NOT_SUPPORTED` 并给出诚实回退。绝不要构建伪造成功状态的处理器。

## 使用方式

1. 先读 `runtime-capability-map.md`，弄清今天能接什么、什么需要补 Host。
2. 从 backlog 取一个任务，一次一个纵向切片（T01、T02……）。
3. 按 backlog 第 3 节报告：改动文件、用到的能力、剩余 `NOT_SUPPORTED`、已运行的测试、风险。

## 维护

审计日期写在 `runtime-capability-map.md`；Host RPC 或 gpui shell 变更后，同步更新对应表格与该日期。V2 HTML 已冻结用于视觉回归，不与代码同步修改。
