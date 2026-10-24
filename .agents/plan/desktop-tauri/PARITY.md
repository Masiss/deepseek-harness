# Parity checklist — native desktop vs web client

A screen is done only when it runs on the native app and its probe passes.
Legend: ✓ transport/API available · ◐ native UI partial · ☐ not started

## Core
- ◐ Session list (`session/list`) — separate `session_list.rs` renderer for projected titles, running/blank markers, relative update age, identity on hover, bounded labels, and arrow-key navigation when no text editor owns keyboard input; navigation bounds and label formatting have unit coverage, but no list-specific native probe yet
- ◐ Session viewer / log (`session/page`) — async read-only page with older-page navigation using Host `beforeSeq` and bounded turn windows; packaged journey reads the current page before and after Host restart, while older-page pagination remains loopback-only
- ✓ Tool-call status derivation — M2
- ✓ Chat submit (`session/prompt`) + follow (`session/follow`) — composer with **file attachment** and **Cancel turn**; history and live events render through the transcript projection; a mock-model run proves prompt → approval → tool → question on the packaged runtime
- ✓ Chat cancel (`session/cancel`) — composer exposes Cancel turn while the assistant is working
- ✓ Follow stream (`session/follow`) — shared authenticated mux with reconnect; snapshot records and live events feed one transcript
- ◐ Transcript projection — durable user/assistant messages, provider reasoning, tool calls and results, approvals, jobs, todo lists, deliverables, goals, plan mode, feedback, and the streaming assistant draft; `transcript_view.rs` headless tests include a 1,000-row transcript constrained to the viewport scroll region
- ✓ Permission approval — `$events` mux + allow-once/reject/delegate; `scripts/approval-probe.sh [allow|reject|delegate]` drives a mock model's escalated bash call through every decision on the packaged runtime: allow runs the command (marker outside the workspace), reject and delegate log the audit outcome and never run it
- ◐ Model selection (`session/modelCatalog`, `session/selectModel`) — composer picker groups providers/models, shows the catalog default, selects the model's default effort, and offers its reasoning efforts; a control initializes the profile default, with packaged Host verification pending
- ◐ Session management (`search`, `create`, `rename`, `fork`) — `packaged-journey-probe.sh` completes a turn against a local mock model, forks the Session, and reads the fork's transcript back after a Host restart; search UI still has no list-level probe

## Tools & terminal
- ✓ Terminal RPC (`environment`, `shells`, `list`, `create`, `rename`, `close`) — M5 transport
- ✓ Terminal streams (`attach`, `retain`) — shared mux; `TerminalWriter` enforces the raw byte budget and the snapshot's encoded-frame window, queueing until acknowledgements
- ◐ Terminal UI — list/create/select/rename/close, attach, resize, input; ANSI SGR colors plus a screen model that honours absolute and relative cursor moves, save/restore, erase-in-line, erase-in-display, wrapping at the terminal width, and a bounded scrollback. `native-journey-probe.sh terminal` creates a Session and a terminal, attaches over the shared mux, writes one command, and reads its output back on the packaged runtime

## Settings & plugins
- ✓ Settings (`describe`, `update`, `replace`, `mutate`, `openSettingsDocument`) — M6
- ✓ Plugin inventory — M6
- ✓ Skills list — `skills/list` in a native tab: name, description, when-to-use, model-invocability, and skill path for the selected session
- ✓ Schedules (`list`, `catalog`, `history`, `delete`, `update`) — M6; per-Session list with delete in egui
- ✓ Workspace files (`read`, `readBytes`, `stat`, `list`) — unary and multipart bytes transport
- ◐ Settings UI — Host namespace/schema/redacted values and guarded path-mutation drafts remain separate from client-local appearance preferences. Native preferences persist theme, zoom, density, theme pack (default, high contrast, Solarized), keyboard shortcuts (settings, marketplace, goals), and default Sessions width in a versioned file, work while disconnected, and have validation/round-trip tests; the shortcuts are consumed by the app shell. Custom font families and the prototype Explorer/source pane are not implemented in egui, and packaged GUI verification of the new controls remains pending. The Host settings UI edit path remains unprobed
- ◐ Plugin management view — inventory, per-entry enable/disable for patch-controlled rows, bundle enable/disable, marketplace search and install, and agent presets; `packaged-journey-probe.sh` installs the same fixture through a loopback npm registry (`installBundle` with an explicit registry), proves it activates, and removes it; marketplace search UI and native UI writes still lack a live probe
- ✓ Workspace, session-control, job, account, remote-access, file-watch, and user-question streams — typed clients over the shared mux; probe verifies workspace/follow, session/control, account/watch, remoteAccess/follow, account/watchExpiry, job/list, and workspaceFiles/changes on a real Host (attachWait still needs a live question; everything else reports a frame)

## App shell
- ✓ Native egui window (no webview) — M7a
- ◐ Native layout — egui headless panel geometry tested at 1024×768, 1440×900, 1920×1080, and 1119×1381; debug probe reports measured panel sizes; no independent visual review
- ✓ Host sidecar launch + cookie auth — M7b
- ✓ Shared Desktop Host lease — `native-lease-probe.sh` proves a second Host is refused and a new Host takes over after owner death
- ✓ Single-instance lock — M7c
- ✓ System tray + product icon + close-to-tray — M7c/M7d
- ✓ Native file dialogs — M7c
- ◐ Native update — Host owns signed metadata verification and atomic AppImage replacement; native presents download/install confirmation and schedules the outer AppImage after the old process exits. `update-flow-probe.mjs` verifies signed Host feed acceptance, artifact replacement, and restart with a mock shell; native prompt handling has unit tests, but a GUI-driven signed swap/restart remains pending
- ◐ AppImage native — rebuilt native AppImage (87,587,320 bytes) passes `appimage-probe.sh`, including profile seeding, packaged Host boot, and baked update/component feed URLs. The native GUI boot/auth/stream probe passes under isolated Weston; Settings controls remain unclicked. The release lane has not been run or published

## Open parity gaps
- ◐ Composer — multiline egui text editing, native clipboard/IME handling, and Ctrl+Enter submission with a unit-tested key predicate; interactive IME and clipboard verification remains pending
- ◐ UI hardening — loading, empty, disconnected, and retry states exist on key screens; fault injection and independent visual review remain pending
- ◐ Document preview — text page, decoded image preview, and Office preview via the Host converter plus local pdftoppm (first page); `office-component-probe.sh` installs the component on the packaged tree, creates a Session, writes a `docx`, and renders it through `officeToPdf/render` (multipart PDF answer); the native preview pane itself is still not driven by a GUI probe
- ✓ Deliverables view — `deliverables/presented` files collect into a panel whose links open the document preview
- ✓ Human interaction — question cards answer `ask_user_question` through the `$events` waterfall (options, multi-select, free text, skip); `userQuestions/answer` transports continued questions; `scripts/approval-probe.sh allow` proves the live round trip. `userQuestions/attachWait` yields no frames because Desktop composes the blocking legacy tool, not the timed one
- ✓ Jobs panel — session job list (`job/list`) with per-job output follow (`job/follow`); the approval probe starts a background job and reads its output through `job/follow` on a real Host
- ◐ Goals, todo, plan — Goal panel creates, edits, pauses, resumes, completes, and clears goals through existing `goals/*` RPCs, and runs `/plan` or `/plan off` through `commands/execute`; recent durable todo, goal, and plan events are shown beside it. `packaged-journey-probe.sh` drives the whole goal lifecycle on the packaged Host and reads a live goal back after a restart, so only an interactive GUI review is missing
- ✓ Remote Event `$events` waterfall transport and approval outcomes; projections/control stream UI pending
- ✓ Multipart binary response decoder and prompt attachment upload (`session/uploadFileBinary`)
- ✓ Native probe (`scripts/native-gui-probe.sh`) launches the packaged runtime under a separate headless Weston compositor, waits for Host readiness, authentication, `$events`, and the stream probe; this run used `--no-measure` and does not establish physical-desktop rendering
- ✓ RSS measurement on a real Hyprland session: 263–266 MiB RSS / 240–245 MiB PSS idle after authentication (one run measured 205/182)

## Memory gate
- ✓ RSS < 300 MiB measured on a real machine (263–265 MiB RSS / 240–245 MiB PSS idle)

## Native protocol and packaged evidence — 2026-10-24

`apps/desktop-tauri/scripts/native-protocol-probe.sh` owns the offline loopback checks documented in `client-core/API.md`. Nine HTTP journeys cover Goal lifecycle/stale revisions, command listing/execution, session management and pagination, terminal lifecycle, settings writes/conflicts, plugin/marketplace results, and multipart document/Office preview. Two additional mux cases cover terminal snapshot/input/ack/output/exit and a question wait's malformed frame/remote failure without interrupting a file watch. The probe passes 112 tests across the library, multipart decoder, and native journey integration test.

The packaged runtime tree and native AppImage are present. L2 passes: Host entry parity/lease, plugin/session/page/settings/fork/goal journey across restart, startup/crash/offline faults, disk-full download, crash during download/extract, Office component and engine resolution, Office-to-PDF conversion, signed Host update flow with a mock shell, Rust PTY and flock providers, image parity, AppImage boot, and the native terminal journey on a headless compositor. A timed-question wait has no live packaged journey: the shipped preset composes the blocking `ask_user_question`, and switching the row to `mode: timed` requires overriding the preset's `config.plugins` array, which changes the model-visible tool surface (invariant 3) and is therefore a product decision, not a probe setting. No production signing key was used and no release was published. The headless Weston run is not L3; physical-desktop rendering, IME/clipboard, UI fault injection, and independent visual review remain open.

`prepare-linux-runtime.ts` now materializes a `node-pty` package that re-exports the Rust provider. The terminal service loads `node-pty` through its own `createRequire`, and that require does not consult the launcher's resolve hooks, so a payload without a resolvable package failed every `terminal/create` with `Cannot find module 'node-pty'` even though the provider itself worked. The native terminal journey above is the regression gate for it.

Marketplace installation consumes the completed `ChangeResult` returned by `installBundle`; `waitForInstall` is reserved for a live request identifier. `client-core` sends all `session/page` bounds supported by the Host; the packaged journey reads current history, while older-page `beforeSeq` requests have loopback coverage only.

Native updater source builds a separately signed `manifest-native.json`, embeds its URL and the component-feed URL in AppRun, verifies framed Host readiness and restart messages, presents download/install confirmation, and schedules the outer AppImage after the old process exits. The signed Host update flow passes with a generated test key and mock shell; production signing/publication and an update initiated from the native GUI remain open.
