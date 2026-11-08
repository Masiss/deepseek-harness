# DSH features → actual Host capability → UI pattern → acceptance

**Evidence scope:** `reference/runtime-capability-map.md` and backlog audited **2026-11-06**. These are source claims, **not** a live audit of the 2026-11-08 repository. Verify method signatures, implementation files and current screenshots before changing code. `Documented` means present in the handoff, not independently tested here.

| Surface / IDs | Documented Host or GPUI path | UX contract | Scope / gap to verify |
|---|---|---|---|
| Workspace / WS-01..07 | `workspaceFiles/*`, tree watcher, local layout | Recent workspace selector, lazy tree, safe preview, access error | Verify real Open Folder in workspace picker, stale rename/conflict and symlink limits |
| Session / SES | `session/*`, restore/drafts; archive/pin documented | `+` + session picker at Explorer foot; switching doesn't kill run | Verify pin/archive UI visibility and list search; hard delete has no RPC |
| Commands / KEY | `commands/list`, `commands/execute`, `command_registry.rs` 14 IDs | Hamburger Application Menu; palette and Quick Open separate | Map all entries to one registry; current bindings may differ by keymap |
| Agent transcript / AG | `session/follow` with cursor; logs plus stdout/stderr/exit/duration | Streaming, tool grouping, scroll anchor, reconnect recovery | `run.status` is inferred from turn/end; no invented canonical global run status |
| Composer / CMP | `@` picker, Queue/Steer, history, slash commands documented | Attach context, model selection, Queue/Steer when supported, stop semantics | Ask/Build modes unsupported; Plan host command only where available |
| Approval / AG-08 | `requestId`, once allow/deny only | Inline scoped command + **Allow once / Deny**; idempotent pending | No `always allow`, expiry or global persistence |
| Changes / CHG | `git/status`, `git/diff`, `git/stage`, `git/unstage`, `git/discard`, `git/commit` | Diff / Files tabs at right, file stats, protected destructive | Binary detection heuristic; no automatic conflict resolution |
| Git / GIT | branches, checkout, state, fetch/pull/push, merge/rebase/cherry-pick/continue/abort | Branch selector, staged status, remote errors, operation banner | No interactive rebase; preserve dirty-worktree guards |
| Terminal / TER | Host PTY create/list/attach/retain/resize/input; GPUI raw keys, 4 panes documented | Bottom dock, detach≠kill, selected-text copy, correct logical paste | No general Ctrl/Alt passthrough or char-level selection in snapshot |
| Preferences / PREF/APP/KEY | local schema v5 + Host `settings/*`; ThemeMode/System, ThemePack, fonts, keymaps | One Preferences window with local-vs-Host save feedback | Some 1.1 preview styles (Midnight/accent trio) are **proposals**, not shipping capability |
| Skills / SKL | `skills/list`, `skills/setDisabled` persisted | source/provider/scope, enable/disable with Host ACK | No version/changelog for skills in snapshot |
| Plugins / EXT | inventory, manager inspect/enable/remove, health | Installed vs enabled, permission disclosure, failure/restart state | No claim of signed/verified badge |
| Marketplace / MKT | index/search/install/update/remove plus manifest permissions/changelog | Actual trust facts, install progress, error/recovery | Update may remove old bundle before install; no rollback RPC |
| Goals/Handoff / T14 | Host logs + local handoff MD generation, workspace guarded write | Generate preview from recorded data, confirm before write | Never auto-run handoff or overwrite path silently |

## Acceptance sequence per vertical slice

1. Inventory concrete GPUI component(s), Host commands and state ownership. Cite code paths, no unsupported guesses.
2. Compare native **1366×768** and **1920×1080** screenshots with `reference/dsh-ui-prototype-v2.html` and annotate deviations.
3. Implement one connected user flow and include idle/loading/empty/error/disconnected/focus/disabled cases as applicable.
4. Test keyboard/IME/paste; verify no fake success, no hidden task cancellation, preserve state on panel hide and session switch.
5. Classify each requested behavior: `SUPPORTED`, `PARTIAL`, `NOT_SUPPORTED` with evidence and any remaining dependency.

## Prioritized UX debt to measure, not assume

D0: source parity; D1: hamburger/workspace/menu; D2: critical state hierarchy in agent timeline; D3: review/diff action scope and conflict; D4: PTY copy/paste and focus; D5: settings/palette and keyboard; D6: extension/marketplace. **Do not implement an unverified gap solely because this table names it.**
