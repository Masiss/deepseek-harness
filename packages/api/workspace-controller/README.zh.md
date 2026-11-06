---
description: "Host 与 Client 工作区控制：修改工作区导航并跟随其完整投影。"
kind: "package-reference"
---
# Workspace Controller

[English](README.md) | 中文

## 概述

`@deepseek-ai/dsh-api-workspace-controller` 拥有 Host 的 `ctx.workspaceController` 服务和生成的 Client `ctx.remote.workspace` namespace。它的 Remote 方法负责创建、重命名、移除和重排 Workspace，在 Workspace 内重排 Session，归档与取消归档 Session，以及跟随完整的 Workspace 投影。当 Client 必须修改或跟随 Workspace 导航时，请通过 API 网关使用它。本包同时拥有 `ctx.directoryPickerController` 与生成的 `ctx.remote.directoryPicker` namespace，因为它承载的选目录 seam 是抽象的，自身从不作为 Loader entry；它还拥有 `ctx.gitController` 与生成的 `ctx.remote.git` namespace，提供 Git 状态、diff、索引变更、提交、分支切换、远端同步和 merge/rebase/cherry-pick/revert 控制。

## 目录

- [使用本包](#use-this-package)
- [模型体验](#model-experience)
- [已知限制与延期工作](#known-limitations-and-deferred-work)
- [开发备注](#dev-note)

-----

<a id="use-this-package"></a>
## 使用本包

Host 控制器会串行执行正确性取决于当前注册表状态的变更，并为预期失败抛出带有稳定错误码的 `RemoteError`。它的 `follow()` 流会同步订阅持久 Workspace 变更，先发出一份完整 baseline，再按顺序发出 `upsert`、`remove`、`order`、`archived` 和 `pinned` 增量。归档与置顶集合都是会话 id 数组，置顶数组把最近置顶的 id 放在前面。重连会以替换 baseline 开始新一代，因此消费方不依赖收到断线期间的每个增量。不带 `stopActivity` 的 `archiveSession` 会以 `workspace/session-active` 拒绝仍有工作在跑的会话，其 details 按族（`turn`、`subagent`、`job`、`schedule`）列出这些工作及各项的 id 与名称；带 `stopActivity: true` 时注册表的提供方先停止这些工作，归档集合持久化后即返回响应，停止在后台收敛。

控制器注入 `remoteAccess`，并经 `typert.resources` 以默认字段 `workspaceId` 登记 `workspace` 资源种类。`workspace/follow` 不读取资源，其调用访问 `global` 资源：principal 不是 owner 的 Peer 只有在其 principal kind 于 [`dsh-api-remote-access`](../remote-access/README.zh.md) 中的 policy 绑定了列出该 action 的 `global` role 时才能订阅它。对这类调用方，baseline、`upsert` 帧与 `order` 中的 id 只保留 `ctx.remoteAccess.allows()` 作为 `workspace` 资源允许的 Workspace，以及它作为 `session` 资源允许的会话 id（各 Workspace 内的会话以及归档与置顶集合），因此 `workspace` role 与 `session` role 也列出同一 action。流记录已下发的 Workspace。某个 Workspace 在下一次变化或新顺序中已不被允许时，流发送一个携带完整过滤后状态的 `replace` 增量帧而不是 `remove`，因为 Client 不会复活已移除的 id；`replace` 是普通增量帧，Client 像开场 baseline 一样整体替换状态，但它不开启新的一代，因此既不报告错误也不重连；访问恢复后，该 Workspace 经普通 `upsert` 重新出现。`remove` 帧表示删除，只发给收到过该 Workspace 的调用方。owner 的帧不过滤。

Client 入口提供 `ClientWorkspaceModel` 和 `createWorkspaceStateStream()`。该模型拥有 Workspace 行、registry 顺序、归档与置顶会话身份、一元变更回显，以及流与一元调用的竞态处理。较新的 Host 行按 `updatedAt` 获胜；已提交的流顺序优先于较旧的一元响应；已经移除的 Workspace id 不会被延迟数据复活。置顶快照仅在会话身份或顺序变化时更新。该包公开与框架无关的快照和订阅，把导航策略与 React 钩子留给 UI owner。`WorkspaceController.archiveSession(sessionId, { stopActivity })` 抛出携带 Host `rpcError` 的 `WorkspaceArchiveError`，界面因此能区分"仍有工作在跑"的拒绝与会话缺失或载体故障，并提议停止这些工作。

<a id="first-use-workspace"></a>
### 首次使用工作区

`workspace.initializeDefault()` 返回持久化的默认工作区；Client service 通过 `workspaces.initializeDefault(signal?)` 提供该操作。它不接受请求参数：固定目录名 `default-workspace` 由 Host 拥有，注册表也以同一路径片段作为初始标题，因此任何语言下同一安装环境都只有一个磁盘路径和一个存储标题。Host 将目录放在其账户的 `<Documents>/deepseek-harness` 下，远程 Web Host 也遵循此规则。操作系统的文件名限制同样适用。Linux 系统查询要求存在 `xdg-user-dir` 且启用了 Documents 目录；不具备该条件的 Host 必须配置 `documentsDirectory` 或使用文件夹选择器。

[Workspace 注册表](../../workspace/workspace/README.zh.md#first-use-workspace)负责资格判断、目录创建和持久化初始化。已有默认工作区直接返回，不再查询 Documents，也不会被重命名或迁移。不满足首次使用条件时返回 `undefined`，启动流程可将目录选择留给用户。查询和创建失败遵循标准 Remote 错误处理。初始化不创建 Session，也不发送消息。

`./default-workspace` 为浏览器消费方导出 `DEFAULT_WORKSPACE_DIRECTORY` 与 `workspaceDisplayTitle(title, localizedDefault)`：仍保留自动标题的工作区按读者语言显示默认名称，其他标题一律原样显示。被用户重命名为 `default-workspace` 的工作区，或从选择器采用的同名文件夹，也会按默认工作区显示；除显示之外没有其他行为依赖该判断。

| 配置 | 默认值 | 用途 |
| --- | --- | --- |
| `documentsDirectory` | 系统 Documents 目录 | 完全限定的 Host 目录覆盖值 |
| `documentsLookupTimeoutMs` | `10000` | 操作系统目录查询的正数最大时长，单位为毫秒 |
| `gitTimeoutMs` | `15000` | 单次本地 `git` 调用的正数最大时长，单位为毫秒 |
| `gitNetworkTimeoutMs` | `120000` | 单次远端 `git` 调用的正数最大时长，单位为毫秒 |
| `gitMaxDiffBytes` | `524288` | 返回的 diff 补丁正数上限；超出时截断并标记 |
| `gitMaxOutputBytes` | `32768` | 返回的远端摘要正数上限；超出时截断并标记 |

Documents 查询占用注册表变更队列，因此其他 Workspace 变更（包括登记已选目录）最多可能等待 `documentsLookupTimeoutMs`。取消可以停止查询；解析成功后，取消不会回滚创建或登记。

<a id="git-namespace"></a>
### Git namespace

每个动词都作用于 Session 工作区根目录所在的仓库。Session 标识以 `gitWorkspaceScopeId` 传输，通过 `gitWorkspaceScope` Typert lookup 从实时或已存储的 Session header 解析；header 没有 cwd 时回退到 sandbox policy 根目录。该 lookup 把 Session 注册为访问控制资源。

`git/status` 解析 `git status --porcelain=v2 -b -z`，返回当前分支、HEAD 提交、detached 标记，以及带封闭变更类型和 staged/worktree 两侧的变更路径。`git/branches` 通过 `git for-each-ref` 列出本地分支。`git/checkout` 只接受已存在的本地分支名，并把 git 自身的拒绝（包括 dirty 工作树）作为 `git/checkout-refused` 返回，不自行改写文件。

`git/diff` 读取单个变更路径的统一补丁：默认比较工作树与索引，`staged: true` 时比较索引与 HEAD；未跟踪路径通过 `git diff --no-index` 读取并标记。超过 `gitMaxDiffBytes` 的补丁会被截断并标记，而不是静默缩短。`git/stage`、`git/unstage` 和 `git/discard` 只接受当前变更且位于仓库内的相对路径：stage 执行 `git add`，unstage 执行 `git restore --staged`，discard 对已跟踪的 modified、deleted 或 typechange 路径执行 `git restore --worktree`。`git/commit` 拒绝空消息，并用该消息提交已暂存索引。

`git/fetch` 执行 `git fetch --all --prune`，`git/pull` 执行 `git pull --ff-only`，`git/push` 执行不带 force 的 `git push`。它们使用 Host 自身的 git credential helper 和 SSH agent；子进程不启用终端提示，因此 Host 无法提供的凭据会以 `git/remote-auth` 失败，而不是挂起。历史被拒绝或分叉返回 `git/remote-diverged`，分支没有 upstream 返回 `git/remote-no-upstream`，其他失败返回 `git/remote-failed`；摘要为 git 自身文本，受 `gitMaxOutputBytes` 限制。

`git/state` 返回进行中的 merge、rebase、cherry-pick 或 revert 及未合并路径。`git/merge` 把一个已存在的本地分支并入当前分支，`git/rebase` 把当前分支重放到该分支上，`git/cherryPick` 应用一个 commit id，`git/revert` 撤销一个 commit。它们都返回结果状态，因此冲突保持可见而不是变成错误。`git/continueOperation` 与 `git/abortOperation` 推进或回退进行中的 merge、rebase、cherry-pick 或 revert；编辑冲突文件并暂存仍由调用方完成。

每个动词都在调用方 signal 以及 `gitTimeoutMs` 或 `gitNetworkTimeoutMs` 约束下以无 shell 方式运行 `git`。不在仓库内的目录返回 `git/not-repository`；缺少可执行文件返回 `git/unavailable`；路径在仓库外或不适用于该动词返回 `git/path-refused`；其他非零退出返回 `git/command-failed`，消息中带有 git 的 stderr；被拒绝的提交返回 `git/commit-refused`；被拒绝的 merge、rebase、cherry-pick、revert、continue 或 abort 返回 `git/operation-refused`。

-----

<a id="model-experience"></a>
## 模型体验

无，因为 Workspace 组织属于浏览器和 Host 的控制状态，并且不注册提示词、工具或会话事件。

#### KV Cache 影响

无直接影响；Workspace 变更不会改变模型请求。

## 已知限制与延期工作

<a id="known-limitations-and-deferred-work"></a>

- Git 动词执行 Host 的 `git` 可执行文件；没有它的 Host 返回 `git/unavailable`。远端动词依赖 Host 账户的 git credential helper 和 SSH agent；本包不保存凭据，也绝不强制 push。
- 交互式 rebase 和自动冲突解决不在范围内：调用方编辑冲突文件、暂存后继续。
- `git/discard` 永不删除未跟踪文件，也不解决冲突；这类请求返回 `git/path-refused`。
- `follow()` 在重连后替换完整投影，不提供持久 cursor 或增量追赶协议。
- 进程内删除标记只会在 Client 模型生命周期内阻止延迟数据复活已移除的 Workspace。


<a id="dev-note"></a>
### 开发备注

<details>
<summary>维护者工作上下文——点击展开</summary>

无。

</details>
