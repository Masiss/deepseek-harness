# Host API inventory

This inventory describes the authenticated `/api` protocol mounted by the Desktop profile. Desktop boots the Web application bundle (`dsh-base` plus `dsh-web-app`), so the rows below are the Desktop rows. A row marked capability-dependent is registered by Desktop but refuses calls when its selected backend is absent. Web-only rows are not included.

## Transport and authentication

Unary calls are `POST /api/<namespace>/<method>` with a JSON `client-request` envelope:

```json
{"type":"client-request","rpcId":"caller-generated UUID","method":"session/page","payload":{"args":{"request":{"address":{"kind":"session","sessionId":"..."},"throughSeq":42}}}}
```

`payload.args` maps the declared Remote method parameter names to their values. The Host answers `server-response`, echoing `rpcId`, with either `{ "ok": true, "value": ... }` or `{ "ok": false, "error": { "code", "message", "details" } }`. A void success may omit `value`. Binary response fields use `multipart/form-data` with a JSON `metadata` field and raw byte parts. Streams use `/api/remote.mux`; each logical stream opens with an `open` frame whose `payload` uses the same `{args: ...}` map, then exchanges `item`, `end`, `cancel`, and server `error` frames correlated by `streamId`.

The browser does not put a token in each RPC envelope. Connection creates or reuses a 32-byte signing secret in the persistent credentials provider under `client-connection/browser-session`, then creates one random process-launch token. The Host adds that token as `?token=` to the application URL. A matching `GET /` mints an HMAC-signed browser-session `dsh-auth-*` cookie and redirects to the clean URL. An answering native Client exchanges the launch token at `GET /connect?token=…&interactions=answer`; the Host mints an API-operator cookie and redirects to `/api/remote.mux`. The browser cookie jar or native HTTP client sends its cookie on API requests and WebSocket upgrades; the Host checks its signature, authority, and expiry. Native Host bridge calls are separate from this inventory; `HttpTransport::post_json_with_timeout` can wait for a bounded Host operation and decodes its HTTP 204 response as null.

## Plain requests

### Headless verification

`bash apps/desktop-tauri/scripts/native-protocol-probe.sh` runs the Rust protocol tests without starting the GUI or a Host. The loopback fixtures use an ephemeral operator cookie and check exact HTTP request envelopes, settings revisions and conflicts, session management, human-command listing/execution, Goal lifecycle, terminal RPCs and mux input/output, plugin/marketplace results, multipart document/Office bytes, and independent question/file streams. They do not access user credentials or install packages. Cargo runs offline and requires cached build dependencies.

These fixtures do not establish packaged-Host parity. `native-journey-probe.sh panels` adds isolated packaged-Host evidence for Marketplace installation/read-back, Office conversion through the native preview pane, and TUI paste/mouse round trips through the attached native terminal panel; `native-journey-probe.sh update-ui` drives the native update prompts through a signed fake feed and verifies the fake AppImage replacement and restart. This is L2 AccessKit coverage, not physical-window interaction or a production-feed test. `session/page` uses the cursor from `session/follow`; the native viewer requests older records with `beforeSeq`, `maxMessages`, and `turnWindow`.

`pluginManager/installBundle` returns the completed change, not an installation receipt. `waitForInstall` returns `null` for an unknown or completed request; `cancelInstall` then returns `{status:"not-running"}`. The native install worker consumes the completed `application` result directly. A market version's optional `install.registry` is passed to both `pluginManager/inspect` and `pluginManager/installBundle`; omitting it leaves the Host's configured registry selection in force.

### Request inventory

Every plain request is a POST unless noted otherwise. `request` and `response` use the TypeScript names from the owning package; the RPC result wrapper is implicit.

| Path | Request | Response | Mount |
|---|---|---|---|
| `session/list` | `{cursor?: string}` | `{items: SessionSummary[]}` | Desktop |
| `session/search` | `{query: string}` | `{items: SessionSearchItem[], hasMore: boolean}` | Desktop |
| `session/create` | `{workspaceId?, cwd?, sessionId?, agentPreset?}` | `{sessionId, agentPreset?}` | Desktop |
| `session/selectModel` | `{sessionId, provider, model, reasoningEffort?}` | `{selected: ModelSelection}` | Desktop |
| `session/initializeDefaultModel` | `{}` | `void` | Desktop |
| `session/modelCatalog` | `{}` | `ModelCatalog` | Desktop |
| `session/canOpenWorkspacePath` | `{}` | `boolean` | Desktop |
| `session/workspaceDesktop` | `{}` | `{name, available, fileManager}` | Desktop |
| `session/openWorkspacePath` | `{path, action?, application?}` | `{opened: true}` | Desktop |
| `session/workspacePathApplications` | `{path}` | `NativeFileApplication[]` | Desktop |
| `session/rename` | `{sessionId, title}` | `{title, seq}` | Desktop |
| `session/fork` | `{sessionId, atSeq?, allowMigration?}` | `{sessionId}` | Desktop |
| `session/prompt` | `{requestId, sessionId, mode, content[], clientTimeZone?}` | `{accepted: true}` | Desktop |
| `session/attachment` | `{sessionId, attachmentId}` | `{attachment, data: string}` | Desktop |
| `session/updateQueue` | `{sessionId, itemId, action}` | `{accepted: true}` | Desktop |
| `session/cancel` | `{sessionId}` | `{accepted: true}` | Desktop |
| `session/page` | `{address, throughSeq, beforeSeq?, maxMessages?, turnWindow?}` | `{records, hasMore}` | Desktop |
| `session/projections` | `{sessionId}` | projection baseline, migration hint, or `null` | Desktop |
| `skills/list` | `{sessionId}` | `{skills: SkillEntry[]}` | Desktop |
| `fileReferences/list` | agent-scoped `{query}` | `FileReferenceCandidate[]` | Desktop |
| `sessionFeedback/record` | `{sessionId, text?, category?}` | recorded or session-not-found result | Desktop |
| `messageFeedback/list` | `{sessionId}` | feedback list result | Desktop |
| `messageFeedback/put` | `{sessionId, messageId, rating, note?, category?, ifVersion}` | feedback item or rejection | Desktop |
| `messageFeedback/delete` | `{sessionId, messageId, ifVersion}` | `{absent: true}` or rejection | Desktop |
| `job/kill` | `{sessionId, jobId}` | `{outcome}` | Desktop |
| `goals/get` | `{agentId}` | `GoalView` or absent value | Desktop |
| `goals/create` | `{agentId, request: {objective, maxGoalRounds?}}` | `{ref: GoalRef}` | Desktop |
| `goals/edit` | `{agentId, ref: GoalRef, request: {objective?, maxGoalRounds?}}` | `GoalView` | Desktop |
| `goals/pause` | `{agentId, ref: GoalRef}` | `GoalView` | Desktop |
| `goals/resume` | `{agentId, ref: GoalRef}` | `GoalView` | Desktop |
| `goals/complete` | `{agentId, ref: GoalRef}` | `GoalView` | Desktop |
| `goals/clear` | `{agentId, ref: GoalRef}` | `GoalRef` tombstone | Desktop |
| `commands/list` | `{agentId}` | `CommandDescriptor[]` | Desktop |
| `commands/execute` | `{agentId, line, submittedAttachments[]}` | `CommandExecution` or absent value | Desktop |
| `workspace/create` | `{path}` | `{workspace, created}` | Desktop |
| `workspace/initializeDefault` | `{}` | workspace or `undefined` | Desktop |
| `workspace/rename` | `{workspaceId, title}` | `{workspace}` | Desktop |
| `workspace/delete` | `{workspaceId}` | `{deleted: true}` | Desktop |
| `workspace/insertBefore` | `{workspaceId, beforeWorkspaceId?}` | `{workspaceIds}` | Desktop |
| `workspace/insertSessionBefore` | `{workspaceId, sessionId, beforeSessionId?}` | `{workspace}` | Desktop |
| `workspace/archiveSession` | `{sessionId, stopActivity?}` | `{archivedSessionIds}` | Desktop |
| `workspace/unarchiveSession` | `{sessionId}` | `{archivedSessionIds}` | Desktop |
| `workspace/pinSession` | `{sessionId}` | `{pinnedSessionIds}` | Desktop |
| `workspace/unpinSession` | `{sessionId}` | `{pinnedSessionIds}` | Desktop |
| `directoryPicker/pick` | `{}` | `string | null` | Desktop, capability-dependent |
| `directoryPicker/list` | `{path?}` | `DirectoryListing` | Desktop, capability-dependent |
| `directoryPicker/createDirectory` | `{path, name}` | `string` | Desktop, capability-dependent |
| `settings/describe` | `{}` | `SettingsDescribeValue` | Desktop |
| `settings/update` | `{ns, patch, expectedRevision?}` | `SettingsNamespaceView` | Desktop |
| `settings/replace` | `{ns, section, expectedRevision?}` | `SettingsNamespaceView` | Desktop |
| `settings/mutate` | `{ns, ops, expectedRevision?}` | `SettingsNamespaceView` | Desktop |
| `settings/openSettingsDocument` | `{}` | `{opened: true}` | Desktop |
| `credentials/describe` | `{refs}` | `Record<string, CredentialInfo>` | Desktop |
| `credentials/set` | `{ref, value}` | `void` | Desktop |
| `credentials/unset` | `{ref}` | `void` | Desktop |
| `pluginInventory/list` | `{}` | `PluginInventorySnapshot` | Desktop |
| `schedule/list` | `{sessionId}` | `ScheduleRecord[]` | Desktop |
| `schedule/catalog` | `{}` | `ScheduleCatalogEntry[]` | Desktop |
| `schedule/history` | `{sessionId, id, limit, before?}` | `ScheduleDeliveryHistoryResult` | Desktop |
| `schedule/delete` | `{sessionId, id}` | `ScheduleDeleteResult` | Desktop |
| `schedule/update` | `{sessionId, id, expected, title?, prompt?, change?}` | `ScheduleUpdateResult` | Desktop |
| `account/getState` | `{}` | `AccountView` | Desktop |
| `account/getProfile` | `AccountClientMetadata` | profile or `null` | Desktop |
| `account/getBalance` | `AccountClientMetadata` | balance or `null` | Desktop |
| `account/getUnnotifiedBonuses` | `AccountClientMetadata` | bonus batch or `null` | Desktop |
| `account/ackBonusNotified` | `{accountId, orderId, client}` | `boolean` | Desktop |
| `account/startSignIn` | `{client, callbackOrigin, loginSource}` | `AccountView` | Desktop |
| `account/cancelSignIn` | `{attemptId}` | `AccountView` | Desktop |
| `account/hasRunningAccountTasks` | `{}` | `boolean` | Desktop |
| `account/signOut` | `AccountClientMetadata` | `AccountView` | Desktop |
| `terminal/environment` | agent-scoped `{}` | `TerminalEnvironment` | Desktop |
| `terminal/shells` | agent-scoped `{}` | `TerminalShell[]` | Desktop |
| `terminal/list` | `{sessionId}` | `WebTerminalInfo[]` | Desktop |
| `terminal/create` | agent-scoped `{shellPath?, id, cols, rows}` | `WebTerminalInfo` | Desktop |
| `terminal/rename` | agent-scoped `{id, title}` | `void` | Desktop |
| `terminal/close` | agent-scoped `{id}` | `void` | Desktop |
| `workspaceFiles/read` | `{workspaceFileScope, path, range}` | `WorkspaceFileText` | Desktop |
| `workspaceFiles/readBytes` | `{workspaceFileScopeId: SessionId, path, options}` | `WorkspaceFileBytes` with bytes | Desktop |
| `workspaceFiles/stat` | `{workspaceFileScope, path}` | `WorkspaceFileStat` | Desktop |
| `workspaceFiles/list` | `{workspaceFileScope, path}` | `WorkspaceDirectoryListing` | Desktop |
| `workspaceFiles/searchPaths` | `{workspaceFileScopeId: SessionId, query, options?}` | `{paths, truncated}` | Desktop |
| `workspaceFiles/rename` | `{workspaceFileScopeId: SessionId, path, destination, expectedVersion?}` | `{previousPath, path, absolutePath, version}` | Desktop |
| `workspaceFiles/delete` | `{workspaceFileScopeId: SessionId, path, expectedVersion?}` | `{path, absolutePath}` | Desktop |
| `/api/file` (`GET`/`HEAD`) | query `path=<absolute path>` | authenticated bounded file `Response` | Desktop exact Fetch route |

## Streams

All streams below use the Gateway WebSocket mux and are cancelled by closing their logical stream. The stream count is **12**.

| Path | Request | Frames |
|---|---|---|
| `session/follow` | `{address, maxMessages?, turnWindow?, assistantStream?}` | `snapshot`, durable `event`, and optional `assistant-stream` |
| `session/control` | `{}` | `baseline`, `projection` |
| `workspace/follow` | `{}` | `baseline`, `replace`, `upsert`, `remove`, `order`, `archived`, `pinned` |
| `job/list` | `{sessionId}` | complete `rows` sets |
| `job/follow` | `{sessionId?, jobId, from?}` | `opened`, `output`, `status` |
| `terminal/retain` | `{sessionId, id}` | `retained` |
| `terminal/attach` | agent-scoped `{agentId, id}` plus one `TerminalInput` per uplink item | `snapshot`, `output`, `state`, `ack` |
| `workspaceFiles/changes` | `{workspaceFileScopeId: SessionId, path}` | `ready`, `change` |
| `account/watchExpiry` | `{}` | `session-expired` |
| `account/watch` | `{}` | `AccountView` snapshots |
| `remoteAccess/follow` | `{}` | `baseline`, `upsert`, `remove` |
| `userQuestions/attachWait` | agent-scoped `{callId}` | `{remainingMs}` |

## Counts and scope

There are **82 plain requests and 12 streams**. The inventory covers the Desktop profile's Web application composition and the protocol-facing registrations found in its Host bundle. It does not cover model tools, Cordis internal events, direct in-process service methods, browser UI slots, upload transport outside Connection, or optional plugins and user-installed profile overlays. It also does not claim that an optional capability is available merely because its controller is mounted.

Goal requests follow the [`GoalService` Remote declarations](../../../packages/goal/goal/src/index.ts), with payload types defined in [`types.ts`](../../../packages/goal/goal/src/types.ts). The `agentId` identifies a live Agent, not a Session lookup parameter. Creation returns only `{ref}`; callers read `goals/get` for the live view. Every edit, phase transition, and clear sends the exact expected `{id, revision}`. `client-core` preserves remote failures without retrying a stale revision. There is no remote `block` or `disarm` method.

Command requests use `agentId` to address the live Session Agent. `commands/execute` records `command/run` and `command/done` and returns an absent value when no command resolves. The native Plan controls invoke the existing `/plan` and `/plan off` commands; Todo edits continue through the model-facing `todo_write` tool, with no direct Todo RPC.

A native client must provide a native presentation for every listed capability. `client-core` implements the `rpcId` envelope, named `args` payloads, API-operator cookie exchange, unary HTTP calls including multipart byte restoration, and a shared physical mux (`RemoteStreamMux`) whose typed clients cover every listed stream: `session/follow`, `session/control`, `workspace/follow`, `job/list`, `job/follow`, `terminal/retain`, `terminal/attach`, `workspaceFiles/changes`, `account/watchExpiry`, `account/watch`, `remoteAccess/follow`, and `userQuestions/attachWait`, plus `$events` for approvals. One socket carries every logical stream; after a carrier loss the open streams fail with a carrier error and the mux replaces the socket with backoff, queueing opens issued in the meantime. The protocol has no per-stream resume, so callers reopen the logical stream. Terminal input honors the session byte budget and the snapshot's encoded-frame window. Prompt file parts reference receipts minted by `POST /api/session/uploadFileBinary?sessionId=…&name=…`, an authenticated raw-byte route outside this inventory.

The egui app provides session management and older-page navigation, chat/follow with file attachments and cancellation, transcript cards and feedback, approval allow/reject/delegate, terminal input and ANSI output, jobs, schedules, skills, settings mutations and update checks, plugin and marketplace management, document previews, Goal lifecycle controls, plan-mode toggling through the Host command registry, and a panel for recent task/plan events. The Host verifies signed metadata and artifact bytes before replacement. `scripts/native-gui-probe.sh` proves boot, authentication, the approval channel, seven stream endpoints, and the memory budget on the packaged runtime. `scripts/approval-probe.sh [allow|reject|delegate]` drives a mock Messages provider through all three approval decisions, reads a live background job over `job/follow`, and answers a live `ask_user_question` through the `$events` waterfall. `userQuestions/answer` supports continued questions; `attachWait` emits frames only for the timed tool, while Desktop composes the blocking legacy tool. Loopback client-core fixtures verify request envelopes but do not claim packaged-Host parity. `native-journey-probe.sh panels` verifies that the native Marketplace UI installs a loopback-registry bundle and reads it back, Office preview renders a real packaged Host conversion, and the terminal panel's paste/mouse bytes reach a TUI through the packaged PTY. `native-journey-probe.sh update-ui` opens the Host update flow from native Settings, clicks both signed-update prompts through AccessKit, and verifies fake-image replacement and restart with an ephemeral test key; the production feed and shipped AppImage remain untested. Goal writes, timed-question waits, physical-window IME/clipboard, UI fault injection, and independent visual review remain unverified. Todo edits continue through the existing `todo_write` tool; no direct Todo RPC is defined. The client must not invent endpoints for operations absent from this inventory.

The native release source generates a separately signed `manifest-native.json` and embeds its feed URL in AppRun. The release lane has not yet rebuilt or published that artifact; update installation should not be considered release-verified until the packaged signed swap/restart journey passes.
