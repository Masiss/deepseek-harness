# Git review surface — prototype v1.3

Status: implemented in the native shell (local commits, not pushed); this file stays the surface and
Host-contract record. The prototype remains the visual reference.
Implementation order, Host prerequisites and gates live in `IMPLEMENTATION_PLAN.md` slices G0–G4.
Base: the frozen `DSH_UI_V2_Approved_Reference.html`, copied and extended; the approved V2 file is
unchanged. `DSH_FEATURE_TRACEABILITY.md` and live Host evidence still own capability claims.

## What the prototype adds

| Area | V2 prototype | v1.3 extension |
|---|---|---|
| Changes header | `Changes · 3 files` + Review all | count, ahead/behind chip, Review all |
| Tabs | Diff, Files | Diff, Files, **History** |
| Files | single flat list, `+24 −12` | grouped **Staged / Changes / Untracked**, kind chip, per-row hover actions (stage, unstage, discard, delete untracked), selection checkboxes, bulk bar, filter |
| Diff | one fixed patch, hunk header + line numbers, Review changes/Discard | per-file header with staged/unstaged switch, hunk cards with old/new gutters, file actions plus per-hunk stage/unstage/discard, binary and untracked states, horizontal scroll, Reviewed toggle |
| Commit | none | commit bar: staged summary, message, recent messages, Amend, Sign-off, Commit, Commit & Push, inline validation |
| History | none | commit **graph** (lanes, merge curves, refs, HEAD ring), commit detail with patch and parent links, cherry-pick/revert entries |
| Operations | none | merge/rebase confirmation, in-progress banner with conflict chips, per-conflict ours/theirs/base choices, Continue/Abort |
| Status bar | branch popup with two demo branches | branch menu (checkout, create, merge, rebase, stash), sync menu (fetch, pull, push, publish upstream) with push confirmation |

Every destructive action opens a confirmation dialog that names the path or hunk and states that the
prototype performs no Git or filesystem work. The UI never reports a Host-confirmed outcome.

## Demo state and transitions

- Files carry hunks with `staged` per hunk, so a path can appear under both Staged and Changes.
- `stage`/`unstage` move hunks between the two sides; a path leaves the panel when it has no hunks.
- `commit` removes staged hunks, appends a History entry, increments ahead, marks HEAD as local
  (Amend becomes available only for a local commit with staged changes), and clears the message.
- `Commit & Push` and the sync push reset ahead to zero and clear the if-pushed Amend hint.
- `discard` removes unstaged hunks after confirmation; deleting an untracked path is presented as a
  separate, stricter confirmation because the Host has no verb for it yet.
- `Reviewed` is keyed by path plus a content signature, so it clears when the file changes.
- Merge/rebase always starts a two-conflict demo operation; resolving both enables Continue.

## History graph

History is a swimlane commit graph, not a flat list:

- Commits arrive newest-first with `parents` and `refs`; the client assigns lanes. Lane
  indexes stay stable for the whole history, so a topic branch keeps its column until it
  merges back, and a lane that waits for an already-listed commit joins that commit's lane
  with an incoming curve.
- Every row shares one gutter width (`--graph-w`, derived from the widest row), so commit
  subjects align. Lanes use theme tokens (`--blue`, `--green`, `--orange`, `--red`) and stay
  legible in both schemes.
- Rows are 40px with subject plus `author · time` and an optional `N parents`; refs render as
  chips after the subject (`main`, `origin/main`, a topic branch, a tag), and HEAD carries a
  ring around its dot.
- `↑`/`↓` move the selection and open the detail; `Enter`/click does the same; the oid column
  is tabular and right-aligned.

The lane layout is client-side. The Host only needs to return each commit's parents and refs
(`git/log`, not implemented yet); `--graph` ASCII output is not required.

## Host mapping

| Prototype action | Existing Host verb | Gap |
|---|---|---|
| status, ahead/behind | `git/status` | ahead/behind, upstream and per-file stats are not returned yet |
| stage/unstage/discard file | `git/stage`, `git/unstage`, `git/discard` | `expectedStatus` exists; untracked delete is refused by design |
| stage/unstage/discard hunk | — | needs a patch-application verb (for example `git/apply` with target `index`/`worktree`) |
| commit, amend, sign-off | `git/commit` | amend and sign-off options missing |
| commit & push, publish | `git/push` | `setUpstream` missing |
| history graph, commit detail | — | needs `git/log` returning `oid`, `parents`, `refs`, author, time and `git/show` for the patch |
| conflict choices, continue, abort | `git/state`, `git/conflict`, `git/resolve`, `git/continueOperation`, `git/abortOperation` | — |
| branch create/rename/delete, stash | `git/checkout`, `git/merge`, `git/rebase`, `git/stash` | create/delete/rename missing |
| fetch, pull, push | `git/fetch`, `git/pull`, `git/push` | — |

## Keyboard and accessibility

- `Ctrl+Shift+G` opens Changes; `Alt+↑/↓` moves between hunks; `s`/`u`/`d` stage, unstage or discard
  the focused hunk (or the selected file).
- `Esc` closes the confirmation dialog and any popup.
- Tabs expose `role=tab` with `aria-selected` and controlled panels; rows are focusable and respond
  to Enter/Space; icon-only actions carry `aria-label` and a tooltip naming the Host verb.

## Verification

Rendered in Chromium 1366×768 and 1920×1080, dark and light, driven through the DevTools protocol:
73 checks covering grouping, filtering, hunk staging, binary and untracked states, commit, amend,
push, graph rows and ref chips, history selection, merge conflict resolution, discard confirmation,
keyboard shortcuts, and a zero-exception/zero-console-error assertion. Screenshots:
`/tmp/dsh-git-proto/`.

The Impeccable detector reports only four `cramped-padding` findings, all on V2's own tab strips
(`buffer-line`, `right-tabs`, `diff-filebar`, `terminal-head`) where children intentionally sit on
the bottom border; contrast and text-size floors inside this surface are fixed.

## Known limits

- Sample data only; no Host connection, no Git execution, no filesystem access.
- Hunk counts and patches are demo fixtures, not parsed from `git/diff`; the history graph uses
  seven demo commits, including one merge, and reads no repository.
- The light theme overrides in this file fix V2's dark-only text colors only for the surfaces this
  extension touches; the rest of V2 keeps its original light-theme behavior.
- `tokens.json` does not exist yet, so these screens still use V2 CSS variables rather than the
  semantic token layer described in `FOUNDATIONS.md`.
