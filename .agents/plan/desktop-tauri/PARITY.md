# Parity checklist — native desktop vs web client

A screen is done only when it runs on the native app and its probe passes.
Legend: ✓ transport/API available · ◐ native UI partial · ☐ not started

## Core
- ◐ Session list (`session/list`) — M2; shown in egui, no native probe yet
- ◐ Session viewer / log (`session/page`) — async read-only page shown in egui; pagination/probe pending
- ✓ Tool-call status derivation — M2
- ◐ Chat submit (`session/prompt`) + follow (`session/follow`) — egui composer with **file attachment** (upload → receipt → prompt file part) and follow display; no real model probe, cancel UI or transcript projection
- ✓ Chat cancel API (`session/cancel`) — transport only; UI cancellation pending
- ✓ Follow stream (`session/follow`) — shared authenticated mux with reconnect
- ◐ Permission approval — `$events` mux + correlated allow-once/reject API; egui approval buttons wired; `$events=ready` proven on a real Host, a decision round-trip still needs a live tool call
- ◐ Model selection (`session/modelCatalog`, `session/selectModel`) — composer picker groups providers/models, shows the catalog default, and reports selection failures; `initializeDefaultModel` transport only
- ◐ Session management (`search`, `create`, `rename`, `fork`) — native controls wired; no live Host probe

## Tools & terminal
- ✓ Terminal RPC (`environment`, `shells`, `list`, `create`, `rename`, `close`) — M5 transport
- ✓ Terminal streams (`attach`, `retain`) — shared mux; `TerminalWriter` enforces the raw byte budget and the snapshot's encoded-frame window, queueing until acknowledgements
- ◐ Terminal UI — list/create/select/rename/close, attach, resize, input; ANSI SGR colors, carriage-return overwrites, and erase-to-end-of-line are rendered; cursor addressing is not emulated; no live Host probe

## Settings & plugins
- ✓ Settings (`describe`, `update`, `replace`, `mutate`, `openSettingsDocument`) — M6
- ✓ Plugin inventory — M6
- ✓ Skills list — M6 (no native surface yet)
- ✓ Schedules (`list`, `catalog`, `history`, `delete`, `update`) — M6; per-Session list with delete in egui
- ✓ Workspace files (`read`, `readBytes`, `stat`, `list`) — unary and multipart bytes transport
- ◐ Settings UI — namespace/schema/redacted values; optimistic user-override patch editor wired, replace/mutate and live Host probe pending
- ◐ Plugin inventory view — module name, enablement, fiber phase, and agent presets; read-only (management mutations and marketplace pending)
- ✓ Workspace, session-control, job, account, remote-access, file-watch, and user-question streams — typed clients over the shared mux; probe verifies workspace/follow, session/control, account/watch, remoteAccess/follow, account/watchExpiry, job/list, and workspaceFiles/changes on a real Host (job/follow and attachWait need a live job or question)

## App shell
- ✓ Native egui window (no webview) — M7a
- ✓ Host sidecar launch + cookie auth — M7b
- ✓ Single-instance lock — M7c
- ✓ System tray + product icon + close-to-tray — M7c/M7d
- ✓ Native file dialogs — M7c
- ◐ Update check — metadata only; installation is intentionally disabled until signed artifact verification exists
- ◐ AppImage AppDir script copies the packaged Host/runtime and supplies AppRun; a self-contained AppDir launch reached `host=ready` and `auth=ok`; not wired into the release lane and not signed

## Open parity gaps
- ◐ Document preview — text page (500 lines, literal selectable) and decoded image preview (PNG/JPEG/GIF/WebP/BMP/ICO up to 16 MiB and 8192 px per side); office preview and Host probe pending
- ☐ Deliverables view
- ☐ Feedback / human interaction
- ◐ Jobs panel — session job list (`job/list`) with per-job output follow (`job/follow`); visible in egui, live-id probe pending
- ☐ Goals, todo, plan panels
- ✓ Remote Event `$events` waterfall transport and approval outcomes; projections/control stream UI pending
- ✓ Multipart binary response decoder and prompt attachment upload (`session/uploadFileBinary`)
- ✓ Native probe (`scripts/native-gui-probe.sh`) launches the packaged runtime, waits for Host readiness, authentication, `$events`, and the stream probe, and reports tree RSS/PSS
- ✓ RSS measurement on a real Hyprland session: 263–266 MiB RSS / 240–245 MiB PSS idle after authentication (one run measured 205/182)

## Memory gate
- ✓ RSS < 300 MiB measured on a real machine (263–265 MiB RSS / 240–245 MiB PSS idle)
