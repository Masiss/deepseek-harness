---
description: "Host and Client workspace control: mutate workspace navigation and follow its complete projection."
kind: "package-reference"
---
# Workspace Controller

English | [中文](README.zh.md)

## Summary

`@deepseek-ai/dsh-api-workspace-controller` owns the Host `ctx.workspaceController` service and the generated Client `ctx.remote.workspace` namespace. Its Remote methods create, rename, remove, and reorder Workspaces, reorder Sessions within a Workspace, archive and unarchive Sessions from Workspace navigation, and follow the Workspace projection. Use it through API Gateway when a Client must change or follow Workspace navigation. The package also owns `ctx.directoryPickerController` and the generated `ctx.remote.directoryPicker` namespace, because the directory-picking seam it carries is abstract and never a Loader entry of its own, and `ctx.gitController` with the generated `ctx.remote.git` namespace for Git state, diffs, patches, index changes, commits, history, checkout, remote synchronization, and merge/rebase/cherry-pick/revert control.

## Table of Contents

- [Use this package](#use-this-package)
- [Model Experience](#model-experience)
- [Known Limitations and Deferred Work](#known-limitations-and-deferred-work)
- [Dev Note](#dev-note)

-----

<a id="use-this-package"></a>
## Use this package

The Host controller serializes mutations whose correctness depends on current registry state and throws `RemoteError` with a stable error code for expected failures. Its `follow()` stream synchronously attaches to durable Workspace changes, emits one complete baseline first, then emits ordered `upsert`, `remove`, `order`, `archived`, and `pinned` increments. Archive and pin sets are Session id arrays, with the most recently pinned id first in the pin array. A reconnect starts another generation with a replacement baseline, so consumers do not depend on receiving every increment while disconnected. `archiveSession` without `stopActivity` refuses a Session with running work as `workspace/session-active`, whose details list that work by family (`turn`, `subagent`, `job`, `schedule`) with item ids and labels; with `stopActivity: true` the registry's providers stop the work first and the response arrives once the archive set is durable, while the stops settle in the background.

The controller injects `remoteAccess` and registers the `workspace` resource kind with the default field `workspaceId` through `typert.resources`. `workspace/follow` reads no resource, so its call touches the `global` resource: a Peer whose principal is not the owner may follow it only when its principal kind's policy in [`dsh-api-remote-access`](../remote-access/README.md) binds a `global` role listing the action. For that caller, the baseline, `upsert` frames, and `order` ids keep only the Workspaces `ctx.remoteAccess.allows()` admits as `workspace` resources, and only the Session ids it admits as `session` resources, within each Workspace and in the archived and pinned sets, so `workspace` and `session` roles list the same action. The stream tracks the Workspaces it delivered. When one is no longer admitted at its next change or in a new order, the stream sends a `replace` increment carrying the complete filtered state instead of a `remove`, because a removed id is never resurrected on the Client; `replace` is an ordinary increment that the Client applies like an opening baseline, not the start of a new generation, so it neither reports an error nor reconnects; once access returns, the Workspace reappears through an ordinary `upsert`. A `remove` frame reports a deletion and reaches only a caller that received that Workspace. Owner frames are unfiltered.

The Client entry provides `ClientWorkspaceModel` and `createWorkspaceStateStream()`. The model owns Workspace rows, registry order, archived and pinned Session identities, unary mutation echoes, and stream/unary race resolution. A newer Host row wins by `updatedAt`; a committed stream order outranks an older unary response; a removed Workspace id cannot be resurrected by delayed data. Pin snapshots change only when their identities or order change. The package exposes framework-neutral snapshots and subscriptions, leaving navigation policy and React hooks to the UI owner. `WorkspaceController.archiveSession(sessionId, { stopActivity })` throws `WorkspaceArchiveError` with the Host's `rpcError`, so a surface can tell the running-work refusal from a missing Session or a carrier fault and offer to stop the work.

<a id="first-use-workspace"></a>
### First-use Workspace

`workspace.initializeDefault()` returns the durable default Workspace; the Client service exposes it as `workspaces.initializeDefault(signal?)`. It takes no request: the Host owns the fixed `default-workspace` directory name, and the registry derives the initial title from that same segment, so one installation keeps one on-disk path and one stored title in every language. The Host places the directory under its account's `<Documents>/deepseek-harness`, including on remote Web hosts. OS filename restrictions apply. Linux system lookup requires `xdg-user-dir` with an enabled Documents directory; hosts without it must configure `documentsDirectory` or use the folder picker.

The [Workspace registry](../../workspace/workspace/README.md#first-use-workspace) owns eligibility, directory creation, and durable initialization. An existing default Workspace is returned without another Documents lookup and is never renamed or relocated. Ineligible first use returns `undefined`, so startup can leave directory selection to the user. Lookup and creation failures use standard Remote error handling. Initialization creates no Session and sends no message.

`DEFAULT_WORKSPACE_DIRECTORY` and `workspaceDisplayTitle(title, localizedDefault)` are published from `./default-workspace` for browser consumers: a Workspace still carrying the automatic title reads as the reader's localized default name, and every other title reads verbatim. A Workspace the user renamed to exactly `default-workspace` — or a folder of that name adopted from the picker — is labeled as the default; nothing else depends on the distinction.

| Configuration | Default | Purpose |
| --- | --- | --- |
| `documentsDirectory` | System Documents directory | Fully qualified Host directory override |
| `documentsLookupTimeoutMs` | `10000` | Positive maximum duration of OS directory lookup, in milliseconds |
| `gitTimeoutMs` | `15000` | Positive maximum duration of one local `git` invocation, in milliseconds |
| `gitNetworkTimeoutMs` | `120000` | Positive maximum duration of one remote `git` invocation, in milliseconds |
| `gitMaxDiffBytes` | `524288` | Positive largest diff patch returned before its tail is dropped and flagged |
| `gitMaxOutputBytes` | `32768` | Positive largest remote summary or history page returned before its tail is dropped and flagged |

Documents lookup holds the registry mutation queue, so other Workspace mutations, including registration of a picked directory, can wait up to `documentsLookupTimeoutMs`. Cancellation can stop the lookup; after resolution succeeds, it does not roll back creation or registration.

<a id="git-namespace"></a>
### Git namespace

Every verb reads the repository that contains a Session's workspace root. The Session identity travels as `gitWorkspaceScopeId` and resolves through the `gitWorkspaceScope` Typert lookup from the live or stored Session header, falling back to the sandbox policy root when the header has no cwd; the lookup registers the Session as the access-control resource.

`git/status` parses `git status --porcelain=v2 -b -z` into the checked-out branch, HEAD commit, detached flag, upstream ref, ahead and behind counts, and changed paths with a closed change kind, the staged and work-tree sides each path carries, a rename or copy source path, and per-side line totals. A branch without upstream reports `upstream: null` with 0 ahead and 0 behind. Line totals come from two `git diff --numstat -z` runs — the index against HEAD and the work tree against the index — and each path reports `stats.index` and `stats.worktree` separately so a path staged and edited again keeps both counts; a side git reports no counts for is absent, and a path with no counted side carries no `stats`. `git/branches` lists local branches from `git for-each-ref`. `git/checkout` accepts only an existing local branch name and surfaces git's own refusal — including a dirty tree — as `git/checkout-refused` without touching files itself.

`git/diff` reads one changed path's unified patch from the work tree or, with `staged: true`, from the index against HEAD; an untracked path is read through `git diff --no-index` and flagged. Patches above `gitMaxDiffBytes` are cut and flagged instead of silently shortened. `git/stage`, `git/unstage`, and `git/discard` accept only a currently changed repository-relative path: stage runs `git add`, unstage runs `git restore --staged`, and discard runs `git restore --worktree` for tracked modified, deleted, or type-changed paths. `git/commit` refuses a blank message and commits the staged index with that message; `amend` replaces the current commit, and an unborn HEAD answers `git/commit-refused`, while `signOff` appends the committer's `Signed-off-by` trailer.

`git/apply` applies one unified patch to the index (`target: "index"`) or the work tree (`target: "worktree"`), backwards with `reverse: true`, which stages, unstages, or discards a hunk without an interactive `git add -p`. The request carries the path, the patch, a target, an optional direction, and the same `expectedStatus` guard as the whole-path verbs. The Host validates the repository-relative path, a non-empty patch within `gitMaxDiffBytes`, and a patch whose `diff --git`, `---`, and `+++` headers name exactly that path and no rename; then it runs `git apply --check` before `git apply`, adding `--cached` for the index and `-R` when reversed. `git/diff` and `git/show` run with `-c core.quotePath=false`, so a patch header names the path byte for byte — including a non-ASCII path — instead of git's C-quoted form. A refused check or apply answers `git/apply-refused` with git's stderr, and a stale status or unsupported change kind answers `git/path-refused`. Git reads the patch from a private 0600 file inside a temporary directory that the verb removes in every outcome, because the no-shell runner writes no child stdin. The result's `operation` is `stage`, `unstage`, or `discard` for those three outcomes and `apply` for a forward work-tree patch.

`git/log` returns commits newest first — `oid`, `parents`, `refs`, `subject`, author name, author email, and an ISO 8601 author instant — from one NUL- and record-separator-delimited `git log` call whose `%D` decorations are normalized to `HEAD`, branch names, remote branches, and `tag:<name>`. `limit` defaults to 50 and is capped at 200, `skip` defaults to 0, and a page past `gitMaxOutputBytes` drops its oldest entries and reports `truncated`. `git/show` reads one commit's patch with `git show --format=`, optionally restricted to one path, cut at `gitMaxDiffBytes` and flagged; it answers the resolved full object id and returns an empty patch for a merge commit, whose combined diff is out of scope. A malformed `limit`, `skip`, or commit id answers `gateway/bad-request`.

`git/fetch` runs `git fetch --all --prune`, `git/pull` runs `git pull --ff-only`, and `git/push` runs `git push` with no force. They use the Host's own git credential helper and SSH agent; children run without terminal prompts, so a credential the Host cannot supply fails as `git/remote-auth` instead of hanging. Rejected or diverged history answers `git/remote-diverged`, a branch without upstream answers `git/remote-no-upstream`, and other failures answer `git/remote-failed`; the summary is git's own text capped by `gitMaxOutputBytes`. With `setUpstream: true`, push publishes the current branch as `git push --set-upstream origin <branch>` and records the upstream; publishing requires a checked-out branch and a repository remote named `origin`, and answers `git/remote-failed` otherwise. An absent `request` field is a plain push, so a caller that sends no options never reaches the publish path.

`git/state` reports the in-progress merge, rebase, cherry-pick, or revert and the unmerged paths. `git/merge` joins an existing local branch into the current one, `git/rebase` replays the current branch onto one, `git/cherryPick` applies one commit id, and `git/revert` undoes one commit. All return the resulting state, so conflicts stay visible instead of collapsing into an error. `git/continueOperation` and `git/abortOperation` advance or unwind the in-progress merge, rebase, cherry-pick, or revert; editing conflicted files and staging them remains the caller's job.

`git/conflict` reads one unmerged path's three index stages: `git ls-files -u` names the stages that exist and each present stage is read with `git show :<stage>:<path>`, so the result carries `base`, `ours`, and `theirs` (stages 1, 2, and 3), `null` for a stage the index does not hold — a delete/modify conflict has two — and the sides `gitMaxDiffBytes` cut. A path that is not unmerged answers `git/path-refused`. `git/resolve` keeps one side with `git checkout --ours|--theirs -- <path>` and stages it with `git add`; a path outside the repository, a path that is not unmerged, and a side the index does not hold answer `git/path-refused`. `git/discardUntracked` deletes one untracked file with `git clean -f -- <path>`, which never touches ignored content and passes no `-d`, so it removes no directory: a path git reports as an untracked directory answers `git/path-refused`, as does a tracked path and a stale `expectedStatus`.

`git/stash` runs one of five actions. `list` reads `git stash list` into entries newest first — entry number, commit id, ISO 8601 save instant, and subject — and a list past `gitMaxOutputBytes` drops its oldest entries and reports `truncated`. `save` runs `git stash push`, recording `message` and adding `-u` for `includeUntracked` but never `-a`, so ignored files stay; a blank message or one longer than 1024 bytes answers `git/stash-refused`. `apply`, `pop`, and `drop` address `stash@{index}`, with `index` defaulting to 0, a negative or fractional number answering `gateway/bad-request`, and an entry git does not have answering `git/stash-refused` with git's stderr. Every result echoes `action`, carries the listed `entries` or git's own `summary` capped by `gitMaxOutputBytes`, and reports `truncated`.

Every verb runs `git` without a shell under the caller's signal plus `gitTimeoutMs` or `gitNetworkTimeoutMs`. A directory outside a repository answers `git/not-repository`; a missing executable answers `git/unavailable`; a path outside the repository, not changed, unmerged where the verb needs it, or otherwise ineligible answers `git/path-refused`; any other nonzero exit answers `git/command-failed` with git's stderr in the message; a refused commit answers `git/commit-refused`; a refused patch answers `git/apply-refused` with git's own diagnostics; a refused merge, rebase, cherry-pick, revert, continue, or abort answers `git/operation-refused`; a refused stash action answers `git/stash-refused` with git's own diagnostics.

-----

<a id="model-experience"></a>
## Model Experience

None, as Workspace organization is browser and Host control state and registers no prompt, tool, or session event.

#### KV Cache effect

No direct effect; Workspace mutations do not alter model requests.

## Known Limitations and Deferred Work

<a id="known-limitations-and-deferred-work"></a>

- Git verbs execute the Host's `git` executable; a host without one answers `git/unavailable`. Remote verbs rely on the Host account's git credential helper and SSH agent; this package stores no credentials and never forces a push.
- Interactive rebase is out of scope, and conflict resolution is per path and per side: `git/resolve` keeps ours or theirs, while a hunk-by-hunk merge still means editing the file and staging it.
- `git/discard` never deletes an untracked file and never resolves a conflict; those requests answer `git/path-refused`. `git/discardUntracked` deletes only an untracked file — never a directory, an ignored file, or a tracked path.
- `git/apply` refuses a patch that names a second path or a rename, and refuses untracked and conflicted paths; those requests answer `git/apply-refused` or `git/path-refused`.
- `git/show` returns no patch for a merge commit; combined diffs are out of scope for the review surface.
- `follow()` replaces the whole projection after reconnect and has no durable cursor or incremental catch-up protocol.
- Process-local deletion markers prevent delayed data from reviving a removed Workspace only for the lifetime of the Client model.


<a id="dev-note"></a>
### Dev Note

<details>
<summary>Working context for maintainers — click to expand</summary>

None.

</details>
