> **V1.2 source precedence:** The untouched approved `reference/dsh-ui-prototype-v2.html` owns visuals and `DSH_FEATURE_TRACEABILITY.md` plus live Host evidence own capability claims. This document is supplementary; it cannot override the frozen V2 layout or claim a feature exists.

# Screen blueprints — DSH product composition

Each screen specifies **primary task / hierarchy / actions / required states / keyboard / evidence**. These are proposed presentation contracts; verify the current Host capabilities before wiring.

## Shell: invariant information architecture

```text
┌ 30px  ☰  Workspace ▾             Search / Commands                ▣ Changes ┐
├ Explorer (~300) ┬ Agent (flex; ≥420 at 100%) ┬ Changes (~565; hideable) ┤
│ tree from top   │ [Agent tab 34px]           │ Changes  • 4 files        │
│ folders/files   │ Session/task context       │ Diff   Files   Branches   │
│                │ Transcript / tool activity │ File / hunk / review      │
│                │ [Composer pinned to bottom]│ Stage / Discard           │
│ + Session ▾     │                            │                           │
├ 26px  branch / Git / sync                 terminal / diagnostics / status ┤
└──────────────────────────────────────────────────────────────────────────┘
```

No activity bar, redundant Explorer header, second Changes tab in center, Settings icon in top/bottom, or duplicate Terminal toggle.

## 1. First run / no workspace

- Main: welcome title, Open Folder, recent workspaces with *actual* path/availability, and privacy/workspace scope explanation.
- Keep shell chrome; no fake tree/chat/file changes.
- States: `no recent`, `permission denied`, `workspace unavailable`, `opening`, `ready`.
- Flow: Open folder → Host approval/picker → real tree → create/recover Session. Failure returns to the same state with reason.

## 2. Explorer + file preview

- Dense tree under top bar. Root one line; twisty, folder/file icon, filename, optional modified marker aligned consistently.
- Folder open lazily; keyboard Left/Right/Up/Down; selected row separate from focused row. Scoped context menu for file operations; dangerous paths carry confirm.
- Click file → safe preview or document view according to capability; title/path, type/size, read-only vs editable, external change banner, Save and version conflict if supported.
- Empty/binary/too-large/denied/deleted/changed states explicit. No blank viewport on failure.

## 3. Session switching

- Bottom Explorer `+` creates one; adjacent current Session button opens search/select popover (recent, pinned, archived when supported). Run dot + text in popover; current session never displayed as a permanent navigation sidebar.
- Switching active run must not cancel implicitly. Save draft/scroll locally. Show clear `Running in another session` only when run is observable.
- Small shortcut path from status/command palette when Explorer hidden. No duplicate session entry points in header.

## 4. Agent timeline and composer

- At top: active Agent tab, concise task/session title and status. Keep task prose as dominant content; assistant replies not enclosed in giant cards.
- Tool activity group begins closed or summarized when resolved, open for error and pending action; show correct count, duration and target. User messages easy to distinguish without large ornamental bubbles.
- Composer: multiline text, @ file context, attachments, model, Queue/Steer (only if actually supported), send/stop; send button disabled reason discoverable. Streaming never steals scroll if user is reading history. Clipboard retains logical newline boundaries.
- States: idle, generating, waiting approval, tool running, tool failed, partially completed, disconnected, run interrupted, retry available, no history.

## 5. Approval dialog/card

- Put actionable approval inline near related tool in timeline or global queue with clear link back to session. Required information: `tool`, `target`, sanitized input/command, reason, read/write risk, exact decision scope.
- Two buttons per historical Host contract: `Allow once` and `Deny`. No unchecked `Always allow`/expiry policy.
- Pending decision prevents double submit. Connection loss does not automatically approve. Modal/dialog content selectable for command inspection.

## 6. Changes and review

- Header real file count and `Review all` only if it performs a defined action. Tabs `Diff`, `Files`, optional `Branches` when product capability exists.
- Diff: file header/path + stats; line gutters; hunk separation; contextual `Stage/Unstage`, `Discard` and review/commit controls; file-level scope distinct from hunk-level.
- Files: unstaged/staged/untracked groups, status text + modified counts, filtering, commit input with validation.
- Branches: actual current branch, fetch/pull/push availability, checkout with dirty-tree guard, current operation and conflicts. Statusbar Git icon opens Changes/Git, not a second repository panel.
- States: no repository, clean tree, loading, binary diff, conflict, stale diff, Git auth error, detached HEAD, operation in progress.

## 7. Terminal bottom panel

- Statusbar icon is the entry. The panel docks **above** statusbar and can be resized or hidden without killing retained Host PTYs. Tabs, selected connection, split panes if supported.
- Terminal copy behavior: soft-wrapped output pasted as logical text, **not** display-line breaks. Respect bracketed paste for actual multiline. Context menu supports copy/paste/select all. Focus and Ctrl+C target carefully: selected text → Copy; no selected text → PTY interrupt if supported.
- States: starting, attached, detached, exited, failed, reconnecting. Distinguish PTY existence from attached stream.

## 8. Preferences

```
┌ Preferences ─────────────────────────────────────────────────────┐
│ General        │ [Current page title]           [page search]    │
│ Appearance     │ Label / help        [control]                    │
│ Keymap         │ ...                                     ...      │
│ Extensions     │ Only relevant sections, clear setting scope     │
│ Skills         │ ...                                              │
│ Marketplace    │ ...                                              │
│ About          │                                   [error/retry]  │
└──────────────────────────────────────────────────────────────────┘
```

- One modal/page window, left navigation 198px preferred, right settings content. Avoid deeply nested cards; settings as aligned rows with immediate help.
- Appearance: theme Dark/Midnight/Light; accent Blue/Teal/Violet; density Compact/Standard/Comfortable; UI scale; font UI/Mono; live preview and **Reset section** only when implemented. Local vs Host persistence labeled.
- Keymap: search commands, current chord, conflict explanation, reset mapping; shortcut shown as platform-specific. Don't claim success until persisted.
- Extensions/Skills: list with scope, installed/enabled status, trust facts, per-item real actions; capability-specific. No `Verified` unless real signed verification.
- Marketplace: index/source, search, publisher + permissions, compatibility, inspect before install, progress and rollback limitations; update treated honestly.
- About/Diagnostics: version, build, Host link state, copy sanitized diagnostic bundle where supported.

## 9. Command palette and search

- One global command query path to search commands, files and sessions (grouped, not collapsed into undifferentiated list). Search type visible; keyboard results stable across async updates.
- Quick open should retain file path and line info; no ambiguous menu row. Null search returns useful recent items, not arbitrary filler.
- States: loading, no results, permission denied, cancelled, multiple workspaces and stale files.

## 10. Screen acceptance checklist

For each implemented screen, review: *primary task identifiable in 3 seconds; one source of truth; visible recoverability; keyboard complete; offline/empty/error state defined; body readable at 125% scale; no clipped popover/focus ring; no fabricated Host result.* Evidence must include native screenshot and tests, not HTML-only screenshots.
