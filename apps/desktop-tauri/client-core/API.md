# Host API inventory

This inventory describes the authenticated `/api` protocol mounted by the Desktop profile. Desktop boots the Web application bundle (`dsh-base` plus `dsh-web-app`), so the rows below are the Desktop rows. A row marked capability-dependent is registered by Desktop but refuses calls when its selected backend is absent. Web-only rows are not included.

## Transport and authentication

Unary calls are `POST /api/<namespace>/<method>` with a JSON `client-request` envelope:

```json
{"type":"client-request","rpcId":"caller-generated UUID","method":"session/page","payload":{"args":{"request":{"address":{"kind":"session","sessionId":"..."},"throughSeq":42}}}}
```

`payload.args` maps the declared Remote method parameter names to their values. The Host answers `server-response`, echoing `rpcId`, with either `{ "ok": true, "value": ... }` or `{ "ok": false, "error": { "code", "message", "details" } }`. A void success may omit `value`. Binary response fields use `multipart/form-data` with a JSON `metadata` field and raw byte parts. Streams use `/api/remote.mux`; each logical stream opens with an `open` frame whose `payload` uses the same `{args: ...}` map, then exchanges `item`, `end`, `cancel`, and server `error` frames correlated by `streamId`.

The browser does not put a token in each RPC envelope. Connection creates or reuses a 32-byte signing secret in the persistent credentials provider under `client-connection/browser-session`, then creates one random process-launch token. The Host adds that token as `?token=` to the application URL. A matching `GET /` mints an HMAC-signed browser-session `dsh-auth-*` cookie and redirects to the clean URL. An answering native Client exchanges the launch token at `GET /connect?token=…&interactions=answer`; the Host mints an API-operator cookie and redirects to `/api/remote.mux`. The browser cookie jar or native HTTP client sends its cookie on API requests and WebSocket upgrades; the Host checks its signature, authority, and expiry.

## Plain requests

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

There are **73 plain requests and 12 streams**. The inventory covers the Desktop profile's Web application composition and the protocol-facing registrations found in its Host bundle. It does not cover model tools, Cordis internal events, direct in-process service methods, browser UI slots, upload transport outside Connection, or optional plugins and user-installed profile overlays. It also does not claim that an optional capability is available merely because its controller is mounted.

A native client must provide a native presentation for every listed capability. `client-core` implements the `rpcId` envelope, named `args` payloads, API-operator cookie exchange, unary HTTP calls including multipart byte restoration, and a shared physical mux (`RemoteStreamMux`) whose typed clients cover every listed stream: `session/follow`, `session/control`, `workspace/follow`, `job/list`, `job/follow`, `terminal/retain`, `terminal/attach`, `workspaceFiles/changes`, `account/watchExpiry`, `account/watch`, `remoteAccess/follow`, and `userQuestions/attachWait`, plus `$events` for approvals. One socket carries every logical stream; after a carrier loss the open streams fail with a carrier error and the mux replaces the socket with backoff, queueing opens issued in the meantime. The protocol has no per-stream resume, so callers reopen the logical stream. Terminal input honors the session byte budget and the snapshot's encoded-frame window. Prompt file parts reference receipts minted by `POST /api/session/uploadFileBinary?sessionId=…&name=…`, an authenticated raw-byte route outside this inventory.

The egui app has a session list/read-only viewer, a prompt/follow panel with approval controls and file attachments, a terminal panel with ANSI rendering, a jobs panel, a redacted settings viewer, and text document preview. `scripts/native-gui-probe.sh` proves boot, authentication, the approval channel, seven stream endpoints, and the memory budget on the packaged runtime; `job/follow` and `userQuestions/attachWait` still need a live job or question, and a complete approval decision round-trip needs a live tool call. Full chat transcript projection, marketplace, image/office preview, deliverables, feedback, goals/todo/plan panels, settings replace/mutate, and the signed updater remain incomplete. It must not invent endpoints for operations absent from this inventory.

The native profile also needs signed update metadata and release-lane AppImage integration before update installation can be enabled. `DSH_AUTO_UPDATE=1` currently remains fail-closed because the feed does not provide a signature-verification contract.
