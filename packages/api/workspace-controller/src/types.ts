/**
 * Browser-safe request, result, and state-stream vocabulary for the Workspace
 * and directory-picking Remote namespaces this package owns. The picking seam
 * declares its own listing types, so they are re-exported here rather than
 * restated: a browser consumer reads the very declaration the backend answers.
 */

import type { SessionId } from '@deepseek-ai/dsh-session/types'
import type { SessionActivity, WorkspaceId } from '@deepseek-ai/dsh-workspace/types'

export type { WorkspaceId } from '@deepseek-ai/dsh-workspace/types'
export type {
  SessionActivity, SessionActivityItem, SessionActivityKind, SessionActivityKindMap,
} from '@deepseek-ai/dsh-workspace/types'
export type { DirectoryEntry, DirectoryListing } from '@deepseek-ai/dsh-host-directory-picker/types'

/** One durable Workspace projected for browser consumers. */
export interface WorkspaceView {
  readonly workspaceId: WorkspaceId
  /** Canonical host directory path. */
  readonly path: string
  /** User-visible title. */
  readonly title: string
  /** Sessions accounted to this Workspace in manual order. */
  readonly sessionIds: readonly SessionId[]
  /** ISO-8601 creation instant. */
  readonly createdAt: string
  /** ISO-8601 last-mutation instant. */
  readonly updatedAt: string
}

declare module '@deepseek-ai/dsh-typert-protocol' {
  interface RemoteErrorDetailsMap {
    /** The requested directory cannot back a Workspace. */
    'workspace/invalid-path': { readonly path: string }
    /** Another Workspace already uses the requested name. */
    'workspace/name-conflict': { readonly name: string }
    /**
     * The Session still has running work — its own turn, a subagent, a
     * background job, or an active schedule — so archiving was refused
     * without a write; `activity` names what must stop first.
     */
    'workspace/session-active': {
      readonly sessionId: SessionId
      readonly activity: readonly SessionActivity[]
    }
    /** The Session or its anchor is not in the Workspace's manual order. */
    'workspace/move-invalid': {
      readonly workspaceId: WorkspaceId
      readonly sessionId: SessionId
      readonly beforeSessionId?: SessionId
    }
    /** The verb needs an interaction the composed backend does not serve. */
    'directory-picker/unavailable': { readonly capability: string }
    /** The target is not fully qualified, or the backend cannot list it. */
    'directory-picker/unreadable': { readonly path: string }
    /** A child of that name is already there. */
    'directory-picker/exists': { readonly path: string }
    /** The parent is not fully qualified, the name is not one segment, or creation failed. */
    'directory-picker/create-failed': { readonly path: string }
    /** The Host has no `git` executable for these verbs. */
    'git/unavailable': Record<string, never>
    /** The Session's workspace root is not inside a git repository. */
    'git/not-repository': { readonly path: string }
    /** `git` exited nonzero for a verb other than checkout. */
    'git/command-failed': { readonly command: string; readonly exitCode: number }
    /** The configured timeout ended the child before it exited. */
    'git/timeout': { readonly timeoutMs: number }
    /** The branch is unknown, the tree refuses the switch, or `git checkout` failed. */
    'git/checkout-refused': { readonly branch: string }
    /** The path is outside the repository, not currently changed, or not eligible for the verb. */
    'git/path-refused': { readonly path: string }
    /** The commit message is blank or git refused the commit. */
    'git/commit-refused': Record<string, never>
    /** The remote action failed for a reason git reported. */
    'git/remote-failed': { readonly operation: GitRemoteOperation }
    /** The remote rejected the Host's credentials. */
    'git/remote-auth': { readonly operation: GitRemoteOperation }
    /** Pull or push would rewrite or diverge from the remote's history. */
    'git/remote-diverged': { readonly operation: GitRemoteOperation }
    /** Push found no upstream branch for the current branch. */
    'git/remote-no-upstream': { readonly operation: GitRemoteOperation }
    /** No multi-step operation is in progress, or git refused the step. */
    'git/operation-refused': Record<string, never>
  }
}

/** Existing directory requested for Workspace adoption. */
export interface WorkspaceCreateRequest {
  readonly path: string
}

/** Created or previously registered Workspace. */
export interface WorkspaceCreateValue {
  readonly workspace: WorkspaceView
  readonly created: boolean
}

/** Workspace title mutation. */
export interface WorkspaceRenameRequest {
  readonly workspaceId: WorkspaceId
  readonly title: string
}

/** Workspace mutation returning the complete changed row. */
export interface WorkspaceValue {
  readonly workspace: WorkspaceView
}

/** Workspace registration deletion. */
export interface WorkspaceDeleteRequest {
  readonly workspaceId: WorkspaceId
}

/** Receipt after one Workspace registration is deleted. */
export interface WorkspaceDeleteValue {
  readonly deleted: true
}

/** DOM-insertBefore-like Workspace order mutation. */
export interface WorkspaceInsertBeforeRequest {
  readonly workspaceId: WorkspaceId
  readonly beforeWorkspaceId?: WorkspaceId
}

/** Complete Workspace registry order after a mutation. */
export interface WorkspaceOrderValue {
  readonly workspaceIds: readonly WorkspaceId[]
}

/** DOM-insertBefore-like Session membership order mutation. */
export interface WorkspaceInsertSessionBeforeRequest {
  readonly workspaceId: WorkspaceId
  readonly sessionId: SessionId
  readonly beforeSessionId?: SessionId
}

/** Session requested for archival from Workspace grouping surfaces. */
export interface WorkspaceArchiveSessionRequest {
  readonly sessionId: SessionId
  /**
   * Stop the Session's running work — its turn, subagent descendants, owned
   * background jobs, and active schedules — instead of refusing the archive
   * as `workspace/session-active`. The stops are requested before the
   * archive write and are not awaited; the response arrives once the archive
   * set is durable.
   */
  readonly stopActivity?: boolean
}

/** Session requested for restoration from the archived Session list. */
export interface WorkspaceUnarchiveSessionRequest {
  readonly sessionId: SessionId
}

/** Complete archived Session set after a mutation. */
export interface WorkspaceArchiveValue {
  readonly archivedSessionIds: readonly SessionId[]
}

/** Session requested for pinning ahead of unpinned Sessions on grouping surfaces. */
export interface WorkspacePinSessionRequest {
  readonly sessionId: SessionId
}

/** Session requested for removal from the pin set. */
export interface WorkspaceUnpinSessionRequest {
  readonly sessionId: SessionId
}

/** Complete pinned Session set after a mutation, most recently pinned first. */
export interface WorkspacePinValue {
  readonly pinnedSessionIds: readonly SessionId[]
}

/** Complete reconnect baseline for Workspace browser state. */
export interface WorkspaceBaseline {
  readonly items: readonly WorkspaceView[]
  readonly archivedSessionIds: readonly SessionId[]
  /** Registry-global pin set, most recently pinned first. */
  readonly pinnedSessionIds: readonly SessionId[]
}

/**
 * One ordered Workspace change after a generation's baseline. `replace`
 * carries the complete state that supersedes everything delivered before it
 * in the generation; it is a change, not a new generation's opening frame.
 */
export type WorkspaceFollowIncrement =
  | { readonly type: 'replace'; readonly value: WorkspaceBaseline }
  | { readonly type: 'upsert'; readonly workspace: WorkspaceView }
  | { readonly type: 'remove'; readonly workspaceId: WorkspaceId }
  | { readonly type: 'order'; readonly workspaceIds: readonly WorkspaceId[] }
  | { readonly type: 'archived'; readonly archivedSessionIds: readonly SessionId[] }
  | { readonly type: 'pinned'; readonly pinnedSessionIds: readonly SessionId[] }

/** Workspace state stream; every generation starts with exactly one baseline. */
export type WorkspaceFollowFrame =
  | { readonly type: 'baseline'; readonly value: WorkspaceBaseline }
  | WorkspaceFollowIncrement

/** Session identity resolved to the repository root Git verbs run against. */
export interface GitWorkspaceScope {
  /** Session identity received on the wire. */
  readonly sessionId: SessionId
  /** Session workspace root, or the deployment fallback when its header has no cwd. */
  readonly workspaceRoot: string
}

/**
 * Kind of change one status record reports. A path that carries both an index
 * and a work-tree change reports the first class git names in its two columns.
 */
export type GitChangeStatus =
  | 'modified'
  | 'added'
  | 'deleted'
  | 'renamed'
  | 'copied'
  | 'typechange'
  | 'untracked'
  | 'ignored'
  | 'conflict'

/** One changed path relative to the repository root. */
export interface GitChangedFile {
  /** Path relative to the repository root, `/`-separated. */
  readonly path: string
  readonly status: GitChangeStatus
  /** Whether the index differs from HEAD, so the path has a staged change. */
  readonly staged: boolean
  /** Whether the work tree differs from the index, so the path has an unstaged change. */
  readonly worktree: boolean
}

/** Working-tree state of the Session's repository. */
export interface GitStatusValue {
  /** Checked-out branch, or null when HEAD is detached. */
  readonly branch: string | null
  /** Full object id of HEAD, or null before the first commit. */
  readonly commit: string | null
  /** Whether HEAD points at a commit rather than a branch. */
  readonly detached: boolean
  /** Changed paths in git's output order. */
  readonly files: readonly GitChangedFile[]
}

/** One local branch. */
export interface GitBranch {
  /** Short branch name. */
  readonly name: string
  /** Full object id the branch points at. */
  readonly commit: string
  /** Whether this branch is checked out. */
  readonly current: boolean
}

/** Local branches of the Session's repository. */
export interface GitBranchesValue {
  /** Local branches in git's reference order. */
  readonly branches: readonly GitBranch[]
  /** Checked-out branch, or null when detached. */
  readonly current: string | null
}

/** Branch requested for checkout. */
export interface GitCheckoutRequest {
  /** Existing local branch name. */
  readonly branch: string
}

/** Result of one checkout. */
export interface GitCheckoutValue {
  /** Branch now checked out. */
  readonly branch: string
}

/** One changed path requested for diff, stage, unstage, or discard. */
export interface GitPathRequest {
  /** Path relative to the repository root, as reported by {@link GitChangedFile.path}. */
  readonly path: string
}

/** Diff request for one changed path. */
export interface GitDiffRequest extends GitPathRequest {
  /** Read the index-against-HEAD diff instead of the work-tree-against-index diff. */
  readonly staged: boolean
}

/** One path's unified diff. */
export interface GitDiffValue {
  /** Path the diff belongs to. */
  readonly path: string
  /** Whether the diff compares the index with HEAD. */
  readonly staged: boolean
  /** Unified diff text with no color or external diff driver. */
  readonly patch: string
  /** Whether the configured byte cap dropped the tail of {@link patch}. */
  readonly truncated: boolean
  /** Whether the path is untracked, in which case the patch comes from `git diff --no-index`. */
  readonly untracked: boolean
}

/** Result of one index or work-tree mutation. */
export interface GitPathValue {
  /** Path the verb applied to. */
  readonly path: string
  /** Verb that settled. */
  readonly operation: 'stage' | 'unstage' | 'discard'
}

/** Commit requested for the staged index. */
export interface GitCommitRequest {
  /** Non-blank commit message. */
  readonly message: string
}

/** Result of a committed index. */
export interface GitCommitValue {
  /** Full object id of the new commit. */
  readonly commit: string
}

/** Explicit remote action run against the repository. */
export type GitRemoteOperation = 'fetch' | 'pull' | 'push'

/** Result of one remote action. */
export interface GitRemoteValue {
  /** Action that ran. */
  readonly operation: GitRemoteOperation
  /** Git's own progress or refusal text, trimmed. */
  readonly summary: string
  /** Whether the configured byte cap dropped the tail of {@link summary}. */
  readonly truncated: boolean
}

/** In-progress multi-step Git operation, or `none`. */
export type GitOperationKind = 'none' | 'merge' | 'rebase' | 'cherry-pick' | 'revert'

/** Multi-step operation state and its unresolved paths. */
export interface GitOperationState {
  /** Operation blocking the work tree, or `none`. */
  readonly operation: GitOperationKind
  /** Paths git reports as unmerged, relative to the repository root. */
  readonly conflicts: readonly string[]
}

/** Branch to rebase the current branch onto. */
export interface GitRebaseRequest {
  /** Existing local branch name. */
  readonly branch: string
}

/** Commit to cherry-pick onto the current branch. */
export interface GitCherryPickRequest {
  /** Abbreviated or full object id. */
  readonly commit: string
}

/** Result of one rebase, cherry-pick, continue, or abort step. */
export interface GitOperationValue {
  /** Operation state after the step. */
  readonly state: GitOperationState
  /** Git's own output for the step, trimmed. */
  readonly output: string
}
