# DSH Runtime Capability Map (Native Desktop)

**Trạng thái:** Audit read-only, không sửa implementation, không đề xuất rewrite host.
**Ngày:** 2026-10-31
**Nguồn:** `DSH_UI_Functional_Spec_v1.0.md`, `dsh-ui-prototype-v2.html`, codebase DSH tại `/home/masis/deepseek-harness`.
**Đối tượng:** Native egui desktop shell (`apps/desktop-tauri/native-app`) qua Host RPC (`apps/desktop-tauri/client-core`).

## 0. Remote transport surface (evidence)

Native app không gọi runtime trực tiếp; đi qua `HttpTransport`/`RemoteStreamMux` trong `apps/desktop-tauri/client-core`. RPC method strings quan sát được:

- Workspace/files: `workspace/follow`, `workspaceFiles/read`, `workspaceFiles/readBytes`, `workspaceFiles/stat`, `workspaceFiles/list`, `workspaceFiles/changes`.
- Session: `session/list`, `session/page`, `session/create`, `session/rename`, `session/fork`, `session/search`, `session/cancel`, `session/prompt`, `session/control`, `session/follow`, `session/initializeDefaultModel`, `session/modelCatalog`, `session/selectModel`, `session/page` (with `throughSeq/beforeSeq/maxMessages/turnWindow`).
- Agent/chat: `session/prompt`, `session/cancel`, `assistant-stream`, `approval/request`, `userQuestions/attachWait`.
- Terminal: `terminal/environment`, `terminal/shells`, `terminal/list`, `terminal/create`, `terminal/rename`, `terminal/close`, `terminal/attach`, `terminal/retain`.
- Settings: `settings/describe`, `settings/update`, `settings/replace`, `settings/mutate`, `settings/openSettingsDocument`.
- Skills: `skills/list`.
- Plugins/extensions/bundles: `pluginInventory/list`, `pluginManager/listPlugins`, `pluginManager/listBundles`, `pluginManager/setPluginEnabled`, `pluginManager/setBundleEnabled`, `pluginManager/removeBundle`.
- Marketplace: `apps/desktop-tauri/client-core/src/marketplace.rs` → `list_indexes`, `search`, `inspect`, `install_bundle`, `wait_for_install`, `cancel_install` (method strings not enumerated here; see file).
- Schedules/goals/models/message feedback/upload also exist but outside scope of this map.

Evidence: `apps/desktop-tauri/client-core/src/{workspace_files,streams,session,sessions,terminal,settings,skills,plugins,marketplace}.rs`.

## 1. Workspace & Explorer

| Spec ID | Capability | Status | Evidence | Gaps |
|---|---|---|---|---|
| WS-01 | Workspace registry (durable records, order, create/delete/move) | Native host (TS) | `packages/workspace/workspace/src/index.ts` (`WorkspaceRegistry`, `create`, `delete`, `insertBefore`, `list`) | No “recent workspaces” RPC; native client only has `workspace/follow` stream and `workspaceFiles/*`. Open-folder flow must be wired through host APIs not exposed as `workspace/open/list` in client-core. |
| WS-02 | Switch workspace, save draft/session context, don't cancel running jobs | Partial | `WorkspaceRegistry` archives/pins sessions; `workspace/session-activity` waterfall (`packages/workspace/workspace/src/index.ts`) | Draft persistence not visible in client-core; workspace switching is session-scoped via `workspaceFileScopeId`, no explicit `workspace.open(rootUri)` RPC in client-core. |
| WS-03 | File tree lazy load, expanded paths per workspace | Partial | `WorkspaceFilesClient::list` (`workspaceFiles/list`) returns direct children + `truncated` (`apps/desktop-tauri/client-core/src/workspace_files.rs`) | No recursive tree endpoint; expansion state is UI-local. No explicit symlink/permission error taxonomy in client-core types (only `truncated`). |
| WS-04 | File preview safe, binary/large handling | Native host | `workspaceFiles/read` (text page with offset/limit/eof), `workspaceFiles/readBytes` (binary page), `workspaceFiles/stat` (`apps/desktop-tauri/client-core/src/workspace_files.rs`) | Binary detection / “open externally” is client-side; no server-side preview kind. |
| WS-05 | Quick open (fuzzy) | Missing | No quick-open RPC in client-core; only `session/search` for sessions | File quick-open not available over Host RPC; must be client-side over `workspaceFiles/list` (costly) or host gap. |
| WS-06 | Sidebar resize/toggle + persisted width | Partial (UI) | Native app `ui_preferences.rs`, `layout.rs` | Persistence of panel widths is client-local; spec wants workspace/user precedence; no RPC for UI state. |
| WS-07 | External file change handling | Native host | `workspaceFiles/changes` stream → `WorkspaceFileWatchFrame { Ready, Change { change: { absolutePath, version, absent } } }` (`apps/desktop-tauri/client-core/src/workspace_files.rs`, `streams.rs`) | Watch is per-path/per-scope; no root-wide watcher; rename/move detection not represented (only absent). |
| WS-08 | File context menu (rename/new/delete) | Missing | No mutating file RPC in client-core | Host gap: no `fs.rename/create/delete` exposed to desktop client. |
| WS-09 | Workspace search (name + content) | Missing | No search RPC in client-core | Host gap. |
| WS-10 | Drag/drop, multi-root, bookmarks | Missing | Not in runtime | Out of scope baseline. |

Also: filesystem sandbox/permission: `packages/fs/fs-sandbox/src/containment.ts`, `packages/fs/fs-local/src/fsio.ts`, `packages/fs/fs-observation-policy`. Evidence of containment exists; symlink handling lives in `packages/workspace/workspace/src/paths.ts` (`realpathNormalize`).

## 2. Session Management

| Spec ID | Capability | Status | Evidence | Gaps |
|---|---|---|---|---|
| SES-01 | Create session in workspace | Native host | `sessions.rs::create` → `session/create` with `workspace_id/cwd/session_id/agent_preset` (`apps/desktop-tauri/client-core/src/sessions.rs`) | Native app `session_list.rs` calls it; OK. |
| SES-02 | Session list/search/dropdown | Native host | `session/list` (cursor), `session/search` (`sessions.rs`) | No archive/pin/filter RPC in client-core (archive/pin are workspace-registry TS, not exposed to native client). |
| SES-03 | Switch session, render history, keep scroll | Native host | `session/page` with `throughSeq/beforeSeq/maxMessages/turnWindow` (`session.rs`) | Turn-window pagination exists; native app manages scroll anchor locally. |
| SES-04 | Restore last workspace/session on startup | Partial | `sessions.rs`, `session/list`; native app settings `restoreLastWorkspace` (client-local) | No host-side “last session” record surfaced; client-local only. |
| SES-05 | Run continues when view detached; reconnect without duplicate run | Native host | `session/follow`, `session/control`, `assistant-stream` (client-core method strings), `remote_stream.rs` | Need native app wiring; `chat.rs::cancel` → `session/cancel`. |
| SES-06 | Rename/archive session | Partial | `session/rename` exists (`sessions.rs`); archive via `WorkspaceRegistry.archiveSession` (TS) | No `session/archive` RPC in client-core; archive is host-internal. Delete not exposed. |
| SES-07 | Fork session from checkpoint | Native host | `session/fork` with `at_seq/allow_migration` (`sessions.rs`) | Native app may not call it yet; check `main.rs`. |
| SES-08 | Search session by title/metadata | Partial | `session/search` returns `sessionId + snippet` only (`sessions.rs`) | No metadata filters/pagination. |
| SES-09 | Export/import/share | Missing | Not in client-core or packages | Out of scope P2. |

Status vocabulary: UI proposes `idle/active/awaiting_approval/detached/interrupted/error/archived`. Host uses `run` status + session events; mapping must be done in adapter. Evidence: `packages/session/session-projection`, `session-stats`; client-core `SessionPage`/`SessionSummary`.

## 3. Agent, Composer, Tool Activity & Approvals

| Spec ID | Capability | Status | Evidence | Gaps |
|---|---|---|---|---|
| AG-01 | Load history by sessionId, roles/order/timestamps | Native host | `session/page` (`session.rs`), `SessionPage.records[].event` (type/data) | Role mapping from event types is client-side. |
| AG-02 | Stream deltas, status queued/streaming/completed/interrupted/failed | Native host | `session/follow`, `assistant-stream` (`client-core`), `remote_stream.rs` | Delta dedup by `runId+sequence` must be implemented in native app; verify in `transcript.rs`/`transcript_view.rs`. |
| AG-03 | Markdown/code copy, path:line links | Partial (UI) | Native `transcript.rs`, `document_preview.rs` | Sanitization of untrusted output required (spec 7.3); check native markdown renderer. |
| AG-04 | Tool activity expand args/stdout/stderr/redacted/duration/exit code | Partial | `session/page` exposes tool calls; `ToolCall { call_id, name, status }` (`session.rs`) | Raw stdout/stderr retrieval not in client-core; redaction happens host-side. Need confirm host returns logs. |
| AG-05 | Tool state queued/running/succeeded/failed/cancelled/unknown | Partial | `ToolCallStatus` enum (inspect `session.rs`) | “unknown” on reconnect must be set by client when no terminal event. |
| AG-06 | Plan/Ask/Build mode | Partial | `session/selectModel` exists; mode not in client-core methods | Mode capability must be verified in host API; client-core has no `mode` method. |
| AG-07 | Stop/Cancel with ack/timeout | Native host | `chat.rs::cancel` → `session/cancel` returns `AcceptedResponse` | Native app must await ack; check `main.rs`. |
| AG-08 | Permission prompt inline/modal | Native host | `approval/request` RPC, `approvals.rs`, `user_questions.rs` (`userQuestions/attachWait`) | Native app has `PendingApproval`/`QuestionCard` (main.rs). |
| AG-09 | Partial/blocked status + Continue | Gap in host? | Agent run status types in core? | No `run.status` enum exposed via client-core; `run.updated` event may carry it. Need inspect `remote_events.rs`. |
| AG-10 | Collapsible thinking only if provided | Not in client-core | No thinking event type in client-core | Host may emit via session events; client must not fabricate. |
| AG-11 | Summarize/handoff | Host capability | `packages/session/session-turn-outline`, `packages/goal` | Not exposed to native client as RPC. |
| AG-12 | Multiple active runs/queue | Not exposed | No queue RPC in client-core | Host policy unknown; client must treat as not supported. |
| CMP-01..05 | Composer, @ mentions, attach, model picker, mode selector | Partial | `models.rs` (catalog/select), `upload.rs` (upload), `session/selectModel` | `@` file picker and attach progress may be client-side; model list from host. |
| CMP-06/07 | clientRequestId idempotency, Send/Queue/Stop | Partial | `chat.rs::prompt` returns `AcceptedResponse` | Idempotency key must be client-generated; verify host honors it (inspect `session-controller`). |
| EV-01..03 | Event stream, reconnect, cursor/resync, no duplicate prompt | Native host | `session/follow`, `follow.rs`, `remote_stream.rs`, `remote_events.rs` (709 lines) | Native app must implement cursor persistence; check `follow.rs`. |

## 4. Changes, Review & Git

| Spec ID | Capability | Status | Evidence | Gaps |
|---|---|---|---|---|
| CHG-01..03 | Changes panel owns Diff/Files, list added/modified/deleted/renamed, hunk diff | Partial | `workspaceFiles/changes` stream only reports file change `{absolutePath, version, absent}`; no diff RPC | Host has `packages/deliverables/workspace-changes` (`git.ts`, `numstat.ts`) for snapshotting, but no `changes.list/diff` RPC exposed to native client. |
| CHG-04..07 | Review state, discard guarded, provenance | Missing in client | No review/discard RPC in client-core | Must be host gap or client-local only; spec warns not to claim success without backend ack. |
| CHG-08..10 | Stage/unstage, commit, checkpoints, compare snapshots | Missing in client | `packages/experimental/worktree` (checkout safety), `packages/deliverables/workspace-changes` | Git operations only internal snapshot support; no user-facing git RPC. |
| GIT-01..04 | Repo detection, branch list, checkout, status refresh | Missing in client | No `git/*` RPC in client-core; `packages/deliverables/workspace-changes/src/git.ts` is internal (rev-parse, snapshotTree) | Native app has no Git UI integration; must be `NOT_SUPPORTED` or implemented via shell tool (agent only), not user Git panel. |
| GIT-05..07 | Stage/commit/fetch/pull/push | Missing | Not exposed | Out of scope unless host adds capability. |

Conclusion: Git/Changes capabilities for a native desktop user are not exposed over Host RPC today; only filesystem watch and file read. Any Changes panel must be backed by host additions or remain read-only file preview.

## 5. Terminal & PTY

| Spec ID | Capability | Status | Evidence | Gaps |
|---|---|---|---|---|
| TER-01 | Open panel, close hides not kill | Partial (UI) | Native app terminal panel in `main.rs` | Process lifetime policy is host-side. |
| TER-02 | PTY attach, cwd per workspace, IO stream, resize | Native host | `terminal/create`, `terminal/attach`, `terminal/retain`, `TerminalInput::Write/Resize`, `TerminalOutputFrame`, `TerminalStateFrame` (`client-core/src/terminal.rs`) | `terminal/create` takes `id, cols, rows, shell_path` but **no cwd**; cwd comes from `terminal/environment` (host-resolved from session). |
| TER-03 | State starting/attached/running/exited/detached/error + exit code | Partial | `TerminalState { Running, Exited, Failed }` only (`client-core/src/terminal.rs`) | Missing `starting/attached/detached/disconnected`; native app must map to these three + error string. |
| TER-04 | Keyboard, paste, select/copy, ANSI, scrollback cap | Partial | `terminal_ansi.rs`, `terminal_input.rs` (native app); `TerminalEnvironment { max_input_bytes, max_buffered_bytes, max_cols, max_rows, scrollback }` | Scrollback cap enforced host-side; client renders. |
| TER-05 | Per-workspace/session consistency, no auto-restart | Partial | `terminal/environment`, `terminal/list`, `terminal/rename`, `terminal/close` | Multi-terminal tabs exist in host (`terminal/list`); native app single panel? check `main.rs`. |
| TER-06 | Multiple terminal tabs + new/split/rename/close with confirm | Native host (list/create/rename/close) | `terminal/list/create/rename/close` | Native UI may not expose tabs yet. |
| TER-07 | Detach/reconnect durable | Native host | `terminal/attach`, `terminal/retain` | `retain` keeps PTY alive when client disconnects (evidence in `terminal.rs` tests). |
| TER-08 | SSH remote terminal | Package exists | `packages/ssh` | Not exposed via terminal RPC; out of scope P2. |

Boundary: agent shell tool (`packages/shell`, `packages/subprocess`) is separate from interactive user PTY; spec 5.1 requires not mixing them. Evidence: `packages/shell/tool-bash`, `packages/subprocess`.

## 6. Preferences

| Spec ID | Capability | Status | Evidence | Gaps |
|---|---|---|---|---|
| PREF-01 | Restore last workspace/session toggle | Partial (client-local) | Native `ui_preferences.rs` (settings panel) | Backed by client-local prefs, not host settings namespace necessarily. |
| PREF-02 | Restore panel layout | Partial (client-local) | `ui_preferences.rs`, `layout.rs` | Client-local only. |
| PREF-03 | Validate, atomic save, reload, error on write fail | Native host (TS settings) | `packages/settings/settings/src/index.ts` (revision conflict `SettingsConflictError`, redact); `settings/describe/update/replace/mutate` with `expected_revision` | Native client uses `settings/*` RPC (generic plugin config), not a dedicated UI prefs store; theme/keymap may not be host namespaces. |
| PREF-04 | Export/import/reset per group | Missing | No export/import RPC | Host gap. |
| APP-01..04 | Theme/accent/density/font scale | Partial (client-local) | Native `ui_preferences.rs`, `settings_panel.rs` | Persisted client-local; no host settings namespace identified. |
| KEY-01..04 | Keymap presets, command registry, conflict validation | Partial (client-local) | Native `ui_preferences.rs` | No host command registry RPC; command registry is client-side. |
| EXT-01..05 | Extension list, enable/disable with runtime ack, permissions, health, failure isolation | Native host (plugins) | `plugins.rs` → `pluginInventory/list`, `pluginManager/listPlugins`, `setPluginEnabled`, `setBundleEnabled`, `removeBundle`; `ChangeResult { application, stage, restart_reason, error, warnings }` | Extension ≠ skill (spec 6.5). Host has `packages/extensions/*` (cordis host/client runner, tool-cordis, ui-cordis). Permission model not visible in client types. |
| SKL-01..03 | Skill list installed, enable/disable, view manifest/provenance | Partial | `skills.rs` → `skills/list` returns `name, description, when_to_use, model_invocable, path` only | No enable/disable, scope, source, version, provenance RPC. Host has `packages/skill/skill` (registry, scope, provenance, providers) and `skill-filesystem`, but only read list exposed. |
| SKL-04/05 | Search/invoke, workspace override, conflicts | Missing in client | Not in `skills.rs` | Host supports scope layers; client cannot toggle. |
| MKT-01..06 | Marketplace search/filter/details/install/update/uninstall/offline | Partial | `marketplace.rs` (list_indexes, search, inspect, install_bundle, wait_for_install, cancel_install) | `installed` vs `enabled` distinction exists in `BundleInfo` (client-core). Trust/verification fields not in client types. |

## 7. Agent status / partial / blocked (cross-cutting)

- `packages/core` agent loop and session events define run lifecycle; native client sees events via `session/follow`/`assistant-stream`. No explicit `run.status` RPC; status must be derived from events. Evidence: `client-core/src/remote_events.rs`, `remote_stream.rs`.
- `packages/session/session-projection`, `session-turn-outline`, `session-checkpoint-policy` provide history/projection; client pages via `session/page`.

## 8. Priority gap summary (P0-focused)

P0 blockers for a truthful native UI:
1. No file mutation RPC (rename/new/delete) and no workspace search/quick-open RPC → WS-08, WS-09, WS-05.
2. No Git/Changes diff/review/discard RPC → all CHG/GIT P0 except file watch; only `workspaceFiles/changes` exists.
3. No session archive/delete/pin RPC in client → SES-06 partial.
4. No mode selector RPC and no run status/queue RPC → AG-06, AG-09, AG-12.
5. Skill enable/disable and provenance not exposed → SKL-01/02/03 partial.
6. Terminal state vocabulary limited to 3 states → TER-03 partial.
7. Settings are generic plugin config; no dedicated UI prefs/theme/keymap host namespace identified → PREF/APP/KEY partial (client-local only).

Capabilities that ARE well-supported and can be wired now: workspace file read/stat/list/watch, session create/list/page/fork/rename/cancel/prompt, approvals/user questions, terminal create/list/attach/retain/resize/close, plugin/extension/bundle enablement, marketplace install lifecycle, settings describe/mutate with revision conflict, model catalog/select.

## 9. Notes

- All findings are evidence-based paths; no code changes were made.
- “Missing in client” means not exposed over Host RPC in `client-core`; host may still have internal capability (TS packages) not reachable by native desktop.
- Per spec §11, unimplemented adapters must be reported as `NOT_SUPPORTED` with truthful UX fallback, not simulated success.
