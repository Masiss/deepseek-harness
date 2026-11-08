# DSH UI design-system v1.2 — prioritized execution

| Phase | Work | Output / gate |
|---|---|---|
| D0, audit only | Read current GPUI/Host, map capability and native screenshots to V2 | Evidence ledger: source path + 1366×768/1920×1080 visual delta, no code changes |
| D1, P0 | Hamburger App Menu, workspace picker, Quick Open/Palette isolation and Esc/focus | Shared command registry, compact anchored menu, real actions, keyboard tests, pixel evidence |
| D2, P0 | Agent transcript/tool/approval/streaming and session state | No fake run completion, once-only allow/deny, recovery, correct focus, scroll, IME |
| D3, P0 | Changes Diff/Files + Git lifecycle | Review staged/unstaged, errors, destructive confirm, preserve scope, conflict states |
| D4, P0 | Terminal PTY and clipboard | Detached process retained, input/raw keys, correct logical paste lines, split panes as supported |
| D5, P0/P1 | Preferences, Keymap and panel persistence | Same window, local-vs-Host settings, validation/persistence, no unapproved theme claims |
| D6, P1 | Skills, extensions and Marketplace | Capability-bound enable/install/update, permissions, no invented verification badges |
| D7, P1 | Native UI audit harness and regression snapshots | Real pixel captures plus geometry and integration tests; not just HTML/`cargo check` |

Use `DSH_FEATURE_TRACEABILITY.md` before estimating. The 2026-11-06 status map is historical; avoid implementing features already present. Review one vertical slice at a time. Do not build a generic design gallery in place of product UX.
