# Parity checklist — native desktop vs web client

A screen is done only when it runs on the native app and its probe passes.
Legend: ✓ transport/API available · ◐ native UI partial · ☐ not started

## Core
- ◐ Session list (`session/list`) — M2; shown in egui, no native probe yet
- ◐ Session viewer / log (`session/page`) — async read-only page shown in egui; pagination/probe pending
- ✓ Tool-call status derivation — M2
- ◐ Chat submit (`session/prompt`) + follow (`session/follow`) — basic egui composer and stream display; no real model probe, cancel UI or transcript projection
- ✓ Chat cancel API (`session/cancel`) — transport only; UI cancellation pending
- ✓ Follow stream (`session/follow`) — authenticated Gateway mux
- ◐ Permission approval — `$events` mux + correlated allow-once/reject API; egui approval buttons wired, needs real Host probe and lifecycle review
- ✓ Model-selection RPCs (`session/modelCatalog`, `session/selectModel`, `session/initializeDefaultModel`) — M4 transport; selector UI pending
- ◐ Session management (`search`, `create`, `rename`, `fork`) — native controls wired; no live Host probe

## Tools & terminal
- ✓ Terminal RPC (`environment`, `shells`, `list`, `create`, `rename`, `close`) — M5 transport
- ✓ Terminal streams (`attach` with single-input uplink, `retain`) — Gateway mux transport; bounded input/backpressure pending
- ◐ Terminal UI — list/create/select/rename/close, attach, resize, input, bounded plain-text output; no ANSI/PTY rendering or live Host probe

## Settings & plugins
- ✓ Settings (`describe`, `update`, `replace`, `mutate`, `openSettingsDocument`) — M6
- ✓ Plugin inventory — M6
- ✓ Skills list — M6
- ✓ Schedules (`list`, `catalog`, `history`, `delete`, `update`) — M6
- ✓ Workspace files (`read`, `readBytes`, `stat`, `list`) — unary and multipart bytes transport
- ◐ Settings UI — namespace/schema/redacted values; optimistic user-override patch editor wired, replace/mutate and live Host probe pending
- ☐ Plugin manager / marketplace UI

## App shell
- ✓ Native egui window (no webview) — M7a
- ✓ Host sidecar launch + cookie auth — M7b
- ✓ Single-instance lock — M7c
- ✓ System tray + product icon + close-to-tray — M7c/M7d
- ✓ Native file dialogs — M7c
- ◐ Update check — metadata only; installation is intentionally disabled until signed artifact verification exists
- ◐ AppImage AppDir script copies the packaged Host/runtime and supplies AppRun; not launched, not wired into release lane, and not signed

## Open parity gaps
- ◐ Text document preview — user-supplied Host path, 500-line page, literal selectable text; images/office preview and Host probe pending
- ☐ Deliverables view
- ☐ Feedback / human interaction
- ☐ Goals, jobs, todo, plan panels
- ✓ Remote Event `$events` waterfall transport and approval outcomes; projections/control stream UI pending
- ✓ Multipart binary response decoder; prompt attachment upload remains pending
- ☐ Probes on the native app
- ☐ RSS measurement of the native app

## Memory gate
- ☐ RSS < 300 MiB measured on a real machine (estimate 240–290 MiB)
