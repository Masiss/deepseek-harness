# Parity checklist — native desktop vs web client

A screen is done only when it runs on the native app and its probe passes.
Legend: ✓ transport/API available · ◐ native UI partial · ☐ not started

## Core
- ◐ Session list (`session/list`) — projected title, running/blank marker, relative update age, identity on hover; no list-specific native probe yet
- ◐ Session viewer / log (`session/page`) — async read-only page shown in egui; pagination/probe pending
- ✓ Tool-call status derivation — M2
- ✓ Chat submit (`session/prompt`) + follow (`session/follow`) — composer with **file attachment** and **Cancel turn**; history and live events render through the transcript projection; a mock-model run proves prompt → approval → tool → question on the packaged runtime
- ✓ Chat cancel API (`session/cancel`) — transport only; UI cancellation pending
- ✓ Follow stream (`session/follow`) — shared authenticated mux with reconnect; snapshot records and live events feed one transcript
- ◐ Transcript projection — durable user/assistant messages, provider reasoning, tool calls and results, approvals, jobs, todo lists, deliverables, goals, plan mode, feedback, and the streaming assistant draft
- ✓ Permission approval — `$events` mux + allow-once/reject/delegate; `scripts/approval-probe.sh [allow|reject|delegate]` drives a mock model's escalated bash call through every decision on the packaged runtime: allow runs the command (marker outside the workspace), reject and delegate log the audit outcome and never run it
- ◐ Model selection (`session/modelCatalog`, `session/selectModel`) — composer picker groups providers/models, shows the catalog default, selects the model's default effort, and offers its reasoning efforts; `initializeDefaultModel` transport only
- ◐ Session management (`search`, `create`, `rename`, `fork`) — native controls wired; no live Host probe

## Tools & terminal
- ✓ Terminal RPC (`environment`, `shells`, `list`, `create`, `rename`, `close`) — M5 transport
- ✓ Terminal streams (`attach`, `retain`) — shared mux; `TerminalWriter` enforces the raw byte budget and the snapshot's encoded-frame window, queueing until acknowledgements
- ◐ Terminal UI — list/create/select/rename/close, attach, resize, input; ANSI SGR colors, carriage-return overwrites, and erase-to-end-of-line are rendered; cursor addressing is not emulated; no live Host probe

## Settings & plugins
- ✓ Settings (`describe`, `update`, `replace`, `mutate`, `openSettingsDocument`) — M6
- ✓ Plugin inventory — M6
- ✓ Skills list — `skills/list` in a native tab: name, description, when-to-use, model-invocability, and skill path for the selected session
- ✓ Schedules (`list`, `catalog`, `history`, `delete`, `update`) — M6; per-Session list with delete in egui
- ✓ Workspace files (`read`, `readBytes`, `stat`, `list`) — unary and multipart bytes transport
- ◐ Settings UI — namespace/schema/redacted values, optimistic draft saved as path mutations (set/unset, sorted, secret slots guarded); no live Host write probe yet
- ◐ Plugin management view — inventory, per-entry enable/disable for patch-controlled rows, bundle enable/disable, marketplace search and install, and agent presets; no live Host write probe yet
- ✓ Workspace, session-control, job, account, remote-access, file-watch, and user-question streams — typed clients over the shared mux; probe verifies workspace/follow, session/control, account/watch, remoteAccess/follow, account/watchExpiry, job/list, and workspaceFiles/changes on a real Host (attachWait still needs a live question; everything else reports a frame)

## App shell
- ✓ Native egui window (no webview) — M7a
- ✓ Host sidecar launch + cookie auth — M7b
- ✓ Single-instance lock — M7c
- ✓ System tray + product icon + close-to-tray — M7c/M7d
- ✓ Native file dialogs — M7c
- ◐ Update check — metadata only; installation is intentionally disabled until signed artifact verification exists
- ✓ AppImage native — `pack-appimage.sh` packs either shell; `DSH_DESKTOP_SHELL=native` produced a native AppImage (87.6 MB) whose AppRun env contract, profile seeding, and packaged Host boot pass `appimage-probe.sh`; the release lane now packs and verifies it, and publishes it as an extra asset

## Open parity gaps
- ◐ Document preview — text page, decoded image preview, and Office preview via the Host converter plus local pdftoppm (first page); no Host probe yet
- ✓ Deliverables view — `deliverables/presented` files collect into a panel whose links open the document preview
- ✓ Human interaction — question cards answer `ask_user_question` through the `$events` waterfall (options, multi-select, free text, skip); `userQuestions/answer` transports continued questions; `scripts/approval-probe.sh allow` proves the live round trip. `userQuestions/attachWait` yields no frames because Desktop composes the blocking legacy tool, not the timed one
- ✓ Jobs panel — session job list (`job/list`) with per-job output follow (`job/follow`); the approval probe starts a background job and reads its output through `job/follow` on a real Host
- ◐ Goals, todo, plan — durable `goal/change`, `todo/write`, and `plan/mode` events render as transcript cards; no dedicated management panel
- ✓ Remote Event `$events` waterfall transport and approval outcomes; projections/control stream UI pending
- ✓ Multipart binary response decoder and prompt attachment upload (`session/uploadFileBinary`)
- ✓ Native probe (`scripts/native-gui-probe.sh`) launches the packaged runtime, waits for Host readiness, authentication, `$events`, and the stream probe, and reports tree RSS/PSS
- ✓ RSS measurement on a real Hyprland session: 263–266 MiB RSS / 240–245 MiB PSS idle after authentication (one run measured 205/182)

## Memory gate
- ✓ RSS < 300 MiB measured on a real machine (263–265 MiB RSS / 240–245 MiB PSS idle)
