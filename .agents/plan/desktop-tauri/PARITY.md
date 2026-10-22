# Parity checklist — native desktop vs web client

A screen is done only when it runs on the native app and its probe passes.
Legend: ✓ transport done · ◐ partial · ☐ not started

## Core
- ✓ Session list (`session/list`) — M2
- ✓ Session viewer / log (`session/page`) — M2
- ✓ Tool-call status derivation — M2
- ✓ Chat submit (`session/prompt`) — M3a
- ✓ Chat cancel (`session/cancel`) — M3a
- ✓ Follow stream (`session/follow`, WS) — M3b
- ◐ Permission approval — request typing done (M4); decisions need the Remote Event channel
- ✓ Model selection (`session/modelCatalog`, `session/selectModel`, `session/initializeDefaultModel`) — M4
- ✓ Session management (`search`, `create`, `rename`, `fork`) — M4

## Tools & terminal
- ✓ Terminal RPC (`environment`, `shells`, `list`, `create`, `rename`, `close`) — M5
- ✓ Terminal streams (`attach` with uplink, `retain`) — M5
- ☐ Terminal UI (selection, resize, render)

## Settings & plugins
- ✓ Settings (`describe`, `update`, `replace`, `mutate`, `openSettingsDocument`) — M6
- ✓ Plugin inventory — M6
- ✓ Skills list — M6
- ✓ Schedules (`list`, `catalog`, `history`, `delete`, `update`) — M6
- ✓ Workspace files (`read`, `readBytes`, `stat`, `list`) — M6
- ☐ Settings UI
- ☐ Plugin manager / marketplace UI

## App shell
- ✓ Native egui window (no webview) — M7a
- ✓ Host sidecar launch + cookie auth — M7b
- ✓ Single-instance lock — M7c
- ✓ System tray + close-to-tray — M7c/M7d
- ✓ Native file dialogs — M7c
- ✓ Update check + opt-in install — M7c/M7d
- ◐ AppImage packaging script — M7 (not wired into release lane, not signed)

## Not started
- ☐ Document preview
- ☐ Deliverables view
- ☐ Feedback / human interaction
- ☐ Goals, jobs, todo, plan panels
- ☐ Remote Event waterfall (approvals, projections)
- ☐ Multipart binary attachment transport
- ☐ Probes on the native app
- ☐ RSS measurement of the native app

## Memory gate
- ☐ RSS < 300 MiB measured on a real machine (estimate 240–290 MiB)
