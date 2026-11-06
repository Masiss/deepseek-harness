/**
 * Host Git Remote owner: repository status, branch listing, and branch
 * checkout for the Session's workspace root.
 *
 * The controller reads the working tree only through `git` executed without a
 * shell, bounded by the caller's signal and a configured timeout. Nothing here
 * guesses a branch or a change set: outside a repository the verbs answer
 * `git/not-repository`, and an executable the deployment does not provide
 * answers `git/unavailable`.
 */

import { existsSync } from 'node:fs'
import { isAbsolute, join } from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { runNativeCommand } from '@deepseek-ai/dsh-native-command'
import type {} from '@deepseek-ai/dsh-sandbox-policy'
import type {} from '@deepseek-ai/dsh-session'
import type {} from '@deepseek-ai/dsh-session-persistence'
import type { SessionId } from '@deepseek-ai/dsh-session/types'
import { Remote, RemoteError, TypertRemoteService, type TypertLookup } from '@deepseek-ai/dsh-typert-protocol'
import type {
  GitBranch,
  GitBranchesValue,
  GitChangedFile,
  GitCheckoutRequest,
  GitCheckoutValue,
  GitCherryPickRequest,
  GitCommitRequest,
  GitCommitValue,
  GitDiffRequest,
  GitDiffValue,
  GitOperationKind,
  GitOperationState,
  GitOperationValue,
  GitPathRequest,
  GitPathValue,
  GitRebaseRequest,
  GitRemoteOperation,
  GitRemoteValue,
  GitStatusValue,
  GitWorkspaceScope,
} from './types.ts'

/** Deployment limits for one Git invocation. */
export interface GitConfig {
  /** Milliseconds one local `git` child may run before it is terminated. */
  readonly timeoutMs: number
  /** Milliseconds one remote `git` child may run before it is terminated. */
  readonly networkTimeoutMs: number
  /** Largest diff patch returned to a caller before the tail is dropped and flagged. */
  readonly maxDiffBytes: number
  /** Largest remote summary returned to a caller before the tail is dropped and flagged. */
  readonly maxOutputBytes: number
}

declare module '@deepseek-ai/cordis' {
  interface Context {
    /** Host owner of the `git` Remote namespace. */
    gitController: GitController
  }
}

declare module '@deepseek-ai/dsh-typert-protocol' {
  interface TypertLookupMap {
    /** Resolve a Session id to its repository root without loading its event body or activating an Agent. */
    gitWorkspaceScope: TypertLookup<GitWorkspaceScope, SessionId>
  }
}

/** Exit evidence for one git child; a nonzero exit is a result, not a rejection. */
export interface GitRunResult {
  readonly stdout: string
  readonly stderr: string
  readonly exitCode: number
}

/** Command boundary for tests: the deployment runner never passes through a shell. */
export type GitRunner = (args: readonly string[], cwd: string, signal: AbortSignal) => Promise<GitRunResult>

/**
 * Reject a command that exited nonzero, preserving the executable's own stderr
 * so the Remote caller can distinguish a missing repository from a refused
 * checkout.
 */
function requireSuccess(result: GitRunResult, what: string): GitRunResult {
  if (result.exitCode !== 0) {
    throw new RemoteError(
      'git/command-failed',
      `${what} failed: ${result.stderr.trim() || `exit ${result.exitCode}`}`,
      { command: what, exitCode: result.exitCode },
    )
  }
  return result
}

/** Git's own wording for a credential the Host cannot supply. */
const AUTH_FAILURE =
  /authentication failed|could not read username|could not read password|permission denied \(publickey\)|terminal prompts disabled/i

/** Environment every Git child sees: no terminal prompts, English diagnostics, and a non-interactive editor. */
const GIT_ENV = {
  GIT_TERMINAL_PROMPT: '0',
  GIT_OPTIONAL_LOCKS: '0',
  GIT_EDITOR: 'true',
  LC_ALL: 'C',
} as const

/**
 * Produce the deployment runner bound to one configured timeout.
 * @param timeoutMs - milliseconds one child may run.
 * @param executable - PATH name or absolute executable; tests replace the name to exercise a missing binary.
 * @returns a runner that never passes through a shell.
 */
export function nativeGitRunner(timeoutMs: number, executable = 'git'): GitRunner {
  return async (args, cwd, signal) => {
    const timeout = AbortSignal.timeout(timeoutMs)
    const combined = AbortSignal.any([signal, timeout])
    return await runNativeCommand(executable, ['-C', cwd, ...args], combined, 'hidden', GIT_ENV).then(
      ({ stdout, stderr }) => ({ stdout, stderr, exitCode: 0 }),
      (error: unknown) => {
        if (signal.aborted) throw new RemoteError('gateway/cancelled', 'git command was cancelled', {}, { cause: error })
        /* v8 ignore next 7 -- a real child that outlives the timeout needs a slow executable; the mapping is fixed. */
        if (timeout.aborted) {
          throw new RemoteError(
            'git/timeout',
            `git command exceeded the ${timeoutMs}ms timeout`,
            { timeoutMs },
            { cause: error },
          )
        }
        const failure = error as { code?: unknown; stdout?: unknown; stderr?: unknown }
        if (failure.code === 'ENOENT') {
          throw new RemoteError('git/unavailable', 'git executable was not found on the Host', {}, { cause: error })
        }
        /* v8 ignore next -- the native runner always attaches string stdio. */
        const stderr = typeof failure.stderr === 'string' ? failure.stderr : ''
        /* v8 ignore next -- the native runner always attaches string stdio. */
        const stdout = typeof failure.stdout === 'string' ? failure.stdout : ''
        const exitCode = typeof failure.code === 'number' ? failure.code : null
        return { stdout, stderr, exitCode: exitCode ?? 1 }
      },
    )
  }
}

/** One parsed line of `git status --porcelain=v2 -b -z`. */
interface StatusHeader {
  branch: string | null
  commit: string | null
  detached: boolean
}

type ChangeKind = GitChangedFile['status']

/** Translate porcelain v2's two-column code into the one status a caller displays. */
function changeKind(xy: string): ChangeKind {
  if (xy === 'UU' || xy === 'AA' || xy === 'DD' || xy === 'AU' || xy === 'UA' || xy === 'DU' || xy === 'UD') {
    return 'conflict'
  }
  const code = xy.replaceAll('.', '').charAt(0)
  switch (code) {
    case 'A': return 'added'
    case 'D': return 'deleted'
    case 'R': return 'renamed'
    case 'C': return 'copied'
    case 'T': return 'typechange'
    case 'M': return 'modified'
    default: return 'modified'
  }
}

/** Path field after `spaces` space separators; undefined when the record is too short. */
function fieldAfter(record: string, spaces: number): string | undefined {
  let index = -1
  for (let seen = 0; seen < spaces; seen += 1) {
    index = record.indexOf(' ', index + 1)
    if (index < 0) return undefined
  }
  const path = record.slice(index + 1)
  return path.length === 0 ? undefined : path
}

/** Read the `-b` header records of porcelain v2 output. */function statusHeader(record: string, header: StatusHeader): void {
  if (record.startsWith('# branch.oid ')) {
    const oid = record.slice('# branch.oid '.length)
    header.commit = oid === '(initial)' ? null : oid
  } else if (record.startsWith('# branch.head ')) {
    const head = record.slice('# branch.head '.length)
    if (head === '(detached)') {
      header.detached = true
      header.branch = null
    } else {
      header.branch = head
    }
  }
}

/**
 * Parse NUL-delimited `git status --porcelain=v2 -b -z` output.
 * @param output - complete stdout of the status command.
 * @returns the branch header and changed paths relative to the repository root.
 * @throws Error when a record is truncated, which indicates incomplete output.
 */
export function parseGitStatus(output: string): GitStatusValue {
  const records = output.split('\0')
  if (records.at(-1) === '') records.pop()
  const header: StatusHeader = { branch: null, commit: null, detached: false }
  const files: GitChangedFile[] = []
  for (let index = 0; index < records.length; index += 1) {
    const record = records[index] as string
    if (record.startsWith('# ')) {
      statusHeader(record, header)
      continue
    }
    const kind = record.charAt(0)
    if (kind === '?') {
      files.push({ path: record.slice(2), status: 'untracked', staged: false, worktree: true })
      continue
    }
    if (kind === '!') {
      files.push({ path: record.slice(2), status: 'ignored', staged: false, worktree: false })
      continue
    }
    if (kind === '1' || kind === '2' || kind === 'u') {
      const fieldCount = kind === '1' ? 9 : kind === '2' ? 10 : 11
      const path = fieldAfter(record, fieldCount - 1)
      if (path === undefined) throw new Error(`truncated git status record: ${record}`)
      const xy = record.slice(2, 4)
      files.push({
        path,
        status: changeKind(xy),
        staged: xy.charAt(0) !== '.',
        worktree: xy.charAt(1) !== '.',
      })
      // A rename record is followed by its original path in the next NUL field.
      if (kind === '2') index += 1
      continue
    }
    throw new Error(`unrecognized git status record: ${record}`)
  }
  return { branch: header.branch, commit: header.commit, detached: header.detached, files }
}

/**
 * Parse `git for-each-ref` output with short name, object id, and HEAD marker.
 * @param output - complete stdout of the branch listing command.
 * @param current - the branch `git status` reported as checked out, when any.
 * @returns local branches in git's reference order.
 */
export function parseGitBranches(output: string, current: string | null): GitBranch[] {
  return output
    .split('\n')
    .filter(line => line.length > 0)
    .map((line) => {
      const [name, commit, head] = line.split('\t')
      if (name === undefined || commit === undefined) throw new Error(`malformed git branch record: ${line}`)
      return { name, commit, current: head === '*' || name === current }
    })
}

/**
 * Normalize one repository-relative path a caller names, refusing absolute
 * paths and parent traversal so a verb can never reach outside the repository.
 */
function repoPath(path: string): string {
  const normalized = path.replaceAll('\\', '/').replace(/^\.\//, '')
  const segments = normalized.split('/')
  if (normalized.length === 0 || normalized.startsWith('/') || /^[A-Za-z]:/.test(normalized)
    || segments.some(segment => segment === '' || segment === '.' || segment === '..')) {
    throw new RemoteError('git/path-refused', `"${path}" is not a repository-relative path`, { path })
  }
  return normalized
}

/** Cut a UTF-8 string to a byte cap without splitting the reported text arbitrarily. */
function truncateUtf8(text: string, maxBytes: number): { text: string; truncated: boolean } {
  const bytes = Buffer.from(text, 'utf8')
  if (bytes.length <= maxBytes) return { text, truncated: false }
  return { text: bytes.subarray(0, maxBytes).toString('utf8'), truncated: true }
}

/** Name the multi-step operation the repository's Git directory records. */
function operationOf(gitDir: string): GitOperationKind {
  if (existsSync(join(gitDir, 'rebase-merge')) || existsSync(join(gitDir, 'rebase-apply'))) return 'rebase'
  if (existsSync(join(gitDir, 'CHERRY_PICK_HEAD'))) return 'cherry-pick'
  if (existsSync(join(gitDir, 'MERGE_HEAD'))) return 'merge'
  if (existsSync(join(gitDir, 'REVERT_HEAD'))) return 'revert'
  return 'none'
}

/**
 * Classify one failed remote command from git's own diagnostics so the Client
 * can offer the action that fits: sign in, reconcile history, or set upstream.
 */
function remoteFailure(stderr: string, operation: GitRemoteOperation): RemoteError {
  if (AUTH_FAILURE.test(stderr)) {
    return new RemoteError('git/remote-auth', `${operation} needs credentials the Host does not have: ${stderr.trim()}`, { operation })
  }
  if (/non-fast-forward|\[rejected\]|fetch first|not possible to fast-forward|diverged/i.test(stderr)) {
    return new RemoteError('git/remote-diverged', `${operation} refused: the branches diverged`, { operation })
  }
  if (/no upstream branch|has no upstream/i.test(stderr)) {
    return new RemoteError('git/remote-no-upstream', `${operation} needs an upstream branch`, { operation })
  }
  return new RemoteError('git/remote-failed', `${operation} failed: ${stderr.trim() || 'git exited nonzero'}`, { operation })
}

/** Git verbs executed against one repository root. */
export class GitCommands {
  /**
   * @param run - local command boundary; its runner adds the configured timeout.
   * @param maxDiffBytes - largest patch this verb returns before it flags truncation.
   * @param networkRun - command boundary for fetch, pull, and push; defaults to `run`.
   * @param maxOutputBytes - largest remote summary returned before it flags truncation.
   */
  constructor(
    private readonly run: GitRunner,
    private readonly maxDiffBytes = 512 * 1024,
    private readonly networkRun: GitRunner = run,
    private readonly maxOutputBytes = 32 * 1024,
  ) {}

  /**
   * Read the repository's branch and working-tree changes.
   * @param root - Session workspace root inside the repository.
   * @param signal - caller cancellation.
   * @returns the checked-out branch and changed paths, or a not-repository failure.
   */
  async status(root: string, signal: AbortSignal): Promise<GitStatusValue> {
    const result = await this.changeSet(root, signal)
    return result
  }

  /**
   * List local branches with the checked-out one marked.
   * @param root - Session workspace root inside the repository.
   * @param signal - caller cancellation.
   * @returns the branches and the checked-out branch name, or null when detached.
   */
  async branches(root: string, signal: AbortSignal): Promise<GitBranchesValue> {
    const status = await this.changeSet(root, signal)
    const result = requireSuccess(
      await this.repo(root, ['for-each-ref', '--format=%(refname:short)%09%(objectname)%09%(HEAD)', 'refs/heads'], signal),
      'git for-each-ref',
    )
    return { branches: parseGitBranches(result.stdout, status.branch), current: status.branch }
  }

  /**
   * Check out one existing local branch.
   * @param root - Session workspace root inside the repository.
   * @param branch - branch name that must already exist locally.
   * @param signal - caller cancellation.
   * @returns the now checked-out branch.
   */
  async checkout(root: string, branch: string, signal: AbortSignal): Promise<GitCheckoutValue> {
    if (branch.length === 0 || branch.startsWith('-') || /[\s]/.test(branch)) {
      throw new RemoteError('git/checkout-refused', `refusing to check out "${branch}"`, { branch })
    }
    const listed = await this.branches(root, signal)
    if (!listed.branches.some(candidate => candidate.name === branch)) {
      throw new RemoteError('git/checkout-refused', `no local branch named "${branch}"`, { branch })
    }
    const result = await this.repo(root, ['checkout', branch], signal)
    if (result.exitCode !== 0) {
      const stderr = result.stderr.trim()
      throw new RemoteError(
        'git/checkout-refused',
        `checking out "${branch}" failed: ${stderr || `exit ${result.exitCode}`}`,
        { branch },
      )
    }
    return { branch }
  }

  /**
   * Read one changed path's unified diff.
   * @param root - Session workspace root inside the repository.
   * @param request - path and whether the staged side is read.
   * @param signal - caller cancellation.
   * @returns the unified patch, capped at the configured byte limit.
   */
  async diff(root: string, request: GitDiffRequest, signal: AbortSignal): Promise<GitDiffValue> {
    const path = repoPath(request.path)
    const change = await this.changeOf(root, path, signal)
    let result: GitRunResult
    if (change.status === 'untracked' && !request.staged) {
      /* v8 ignore next -- the device name follows one host platform. */
      const nullDevice = process.platform === 'win32' ? 'NUL' : '/dev/null'
      result = await this.repo(root, ['diff', '--no-color', '--no-ext-diff', '--unified=3', '--no-index', '--', nullDevice, path], signal)
      // `--no-index` exits 1 when the files differ, which is the expected outcome here.
      if (result.exitCode !== 0 && result.exitCode !== 1) requireSuccess(result, 'git diff')
    } else {
      result = requireSuccess(
        await this.repo(root, [
          'diff', '--no-color', '--no-ext-diff', '--unified=3',
          ...(request.staged ? ['--cached'] : []),
          '--', path,
        ], signal),
        'git diff',
      )
    }
    const bytes = Buffer.from(result.stdout, 'utf8')
    const truncated = bytes.length > this.maxDiffBytes
    return {
      path,
      staged: request.staged,
      patch: truncated ? bytes.subarray(0, this.maxDiffBytes).toString('utf8') : result.stdout,
      truncated,
      untracked: change.status === 'untracked',
    }
  }

  /**
   * Stage one changed path into the index.
   * @param root - Session workspace root inside the repository.
   * @param request - path to stage.
   * @param signal - caller cancellation.
   * @returns the settled path and verb.
   */
  async stage(root: string, request: GitPathRequest, signal: AbortSignal): Promise<GitPathValue> {
    const path = repoPath(request.path)
    await this.changeOf(root, path, signal)
    requireSuccess(await this.repo(root, ['add', '--', path], signal), 'git add')
    return { path, operation: 'stage' }
  }

  /**
   * Remove one path's staged change while keeping the work tree untouched.
   * @param root - Session workspace root inside the repository.
   * @param request - path with a staged change.
   * @param signal - caller cancellation.
   * @returns the settled path and verb.
   */
  async unstage(root: string, request: GitPathRequest, signal: AbortSignal): Promise<GitPathValue> {
    const path = repoPath(request.path)
    const change = await this.changeOf(root, path, signal)
    if (!change.staged) {
      throw new RemoteError('git/path-refused', `"${path}" has no staged change`, { path })
    }
    requireSuccess(await this.repo(root, ['restore', '--staged', '--', path], signal), 'git restore --staged')
    return { path, operation: 'unstage' }
  }

  /**
   * Restore one tracked path's work-tree content from the index.
   * @param root - Session workspace root inside the repository.
   * @param request - path with an unstaged change.
   * @param signal - caller cancellation.
   * @returns the settled path and verb.
   */
  async discard(root: string, request: GitPathRequest, signal: AbortSignal): Promise<GitPathValue> {
    const path = repoPath(request.path)
    const change = await this.changeOf(root, path, signal)
    if (!change.worktree) {
      throw new RemoteError('git/path-refused', `"${path}" has no unstaged change to discard`, { path })
    }
    if (change.status === 'untracked' || change.status === 'ignored' || change.status === 'added'
      || change.status === 'renamed' || change.status === 'copied' || change.status === 'conflict') {
      throw new RemoteError('git/path-refused', `"${path}" cannot discard its ${change.status} change safely`, { path })
    }
    requireSuccess(await this.repo(root, ['restore', '--worktree', '--', path], signal), 'git restore')
    return { path, operation: 'discard' }
  }

  /**
   * Commit the staged index with one message.
   * @param root - Session workspace root inside the repository.
   * @param request - commit message.
   * @param signal - caller cancellation.
   * @returns the new commit's full object id.
   */
  async commit(root: string, request: GitCommitRequest, signal: AbortSignal): Promise<GitCommitValue> {
    const message = request.message.trim()
    if (message.length === 0) {
      throw new RemoteError('git/commit-refused', 'commit message is blank', {})
    }
    const result = await this.repo(root, ['commit', '-m', message], signal)
    if (result.exitCode !== 0) {
      throw new RemoteError(
        'git/commit-refused',
        `git commit failed: ${result.stderr.trim() || `exit ${result.exitCode}`}`,
        {},
      )
    }
    const head = requireSuccess(await this.repo(root, ['rev-parse', 'HEAD'], signal), 'git rev-parse')
    return { commit: head.stdout.trim() }
  }

  /**
   * Fetch every remote and prune deleted references.
   * @param root - Session workspace root inside the repository.
   * @param signal - caller cancellation.
   * @returns git's own progress summary.
   */
  async fetch(root: string, signal: AbortSignal): Promise<GitRemoteValue> {
    return await this.remote(root, ['fetch', '--all', '--prune'], 'fetch', signal)
  }

  /**
   * Fast-forward the current branch from its upstream, never creating a merge commit.
   * @param root - Session workspace root inside the repository.
   * @param signal - caller cancellation.
   * @returns git's own progress summary.
   */
  async pull(root: string, signal: AbortSignal): Promise<GitRemoteValue> {
    return await this.remote(root, ['pull', '--ff-only'], 'pull', signal)
  }

  /**
   * Push the current branch to its upstream without force.
   * @param root - Session workspace root inside the repository.
   * @param signal - caller cancellation.
   * @returns git's own progress summary.
   */
  async push(root: string, signal: AbortSignal): Promise<GitRemoteValue> {
    return await this.remote(root, ['push'], 'push', signal)
  }

  /**
   * Read the repository's multi-step operation and unmerged paths.
   * @param root - Session workspace root inside the repository.
   * @param signal - caller cancellation.
   * @returns the operation kind and conflict paths.
   */
  async state(root: string, signal: AbortSignal): Promise<GitOperationState> {
    const directory = requireSuccess(
      await this.repo(root, ['rev-parse', '--git-dir'], signal),
      'git rev-parse',
    ).stdout.trim()
    const conflicts = requireSuccess(
      await this.repo(root, ['diff', '--name-only', '--diff-filter=U', '-z'], signal),
      'git diff',
    ).stdout
    return {
      operation: operationOf(isAbsolute(directory) ? directory : join(root, directory)),
      conflicts: conflicts.split('\0').filter(path => path.length > 0),
    }
  }

  /**
   * Rebase the current branch onto one existing local branch.
   * @param root - Session workspace root inside the repository.
   * @param request - branch to rebase onto.
   * @param signal - caller cancellation.
   * @returns the resulting operation state; conflicts keep the rebase in progress.
   */
  async rebase(root: string, request: GitRebaseRequest, signal: AbortSignal): Promise<GitOperationValue> {
    const branch = request.branch
    if (branch.length === 0 || branch.startsWith('-') || /\s/.test(branch)) {
      throw new RemoteError('git/operation-refused', `refusing to rebase onto "${branch}"`, {})
    }
    const listed = await this.branches(root, signal)
    if (!listed.branches.some(candidate => candidate.name === branch)) {
      throw new RemoteError('git/operation-refused', `no local branch named "${branch}"`, {})
    }
    return await this.step(root, ['rebase', branch], signal)
  }

  /**
   * Cherry-pick one commit onto the current branch.
   * @param root - Session workspace root inside the repository.
   * @param request - abbreviated or full object id.
   * @param signal - caller cancellation.
   * @returns the resulting operation state; conflicts keep the pick in progress.
   */
  async cherryPick(root: string, request: GitCherryPickRequest, signal: AbortSignal): Promise<GitOperationValue> {
    if (!/^[0-9a-f]{4,64}$/i.test(request.commit)) {
      throw new RemoteError('git/operation-refused', `"${request.commit}" is not a commit id`, {})
    }
    return await this.step(root, ['cherry-pick', request.commit], signal)
  }

  /**
   * Continue the in-progress rebase or cherry-pick after conflicts are staged.
   * @param root - Session workspace root inside the repository.
   * @param signal - caller cancellation.
   * @returns the resulting operation state.
   */
  async continueOperation(root: string, signal: AbortSignal): Promise<GitOperationValue> {
    const state = await this.state(root, signal)
    switch (state.operation) {
      case 'rebase':
        return await this.step(root, ['rebase', '--continue'], signal)
      case 'cherry-pick':
        return await this.step(root, ['cherry-pick', '--continue'], signal)
      default:
        throw new RemoteError('git/operation-refused', 'no rebase or cherry-pick is in progress', {})
    }
  }

  /**
   * Abort the in-progress rebase or cherry-pick and restore the previous HEAD.
   * @param root - Session workspace root inside the repository.
   * @param signal - caller cancellation.
   * @returns the restored operation state.
   */
  async abortOperation(root: string, signal: AbortSignal): Promise<GitOperationValue> {
    const state = await this.state(root, signal)
    switch (state.operation) {
      case 'rebase':
        return await this.step(root, ['rebase', '--abort'], signal)
      case 'cherry-pick':
        return await this.step(root, ['cherry-pick', '--abort'], signal)
      default:
        throw new RemoteError('git/operation-refused', 'no rebase or cherry-pick is in progress', {})
    }
  }

  private async remote(
    root: string,
    args: readonly string[],
    operation: GitRemoteOperation,
    signal: AbortSignal,
  ): Promise<GitRemoteValue> {
    const result = await this.networkRun(args, root, signal)
    if (result.exitCode !== 0) throw remoteFailure(result.stderr, operation)
    const summary = truncateUtf8(result.stderr.trim() || result.stdout.trim(), this.maxOutputBytes)
    return { operation, summary: summary.text, truncated: summary.truncated }
  }

  private async step(root: string, args: readonly string[], signal: AbortSignal): Promise<GitOperationValue> {
    const result = await this.repo(root, args, signal)
    const state = await this.state(root, signal)
    if (result.exitCode !== 0 && state.conflicts.length === 0) {
      throw new RemoteError('git/operation-refused', result.stderr.trim() || `exit ${result.exitCode}`, {})
    }
    return { state, output: result.stderr.trim() || result.stdout.trim() }
  }

  /** Require that `path` is currently changed and report how. */
  private async changeOf(root: string, path: string, signal: AbortSignal): Promise<GitChangedFile> {
    const change = (await this.changeSet(root, signal)).files.find(candidate => candidate.path === path)
    if (change === undefined) {
      throw new RemoteError('git/path-refused', `"${path}" is not a currently changed path`, { path })
    }
    return change
  }

  private async changeSet(root: string, signal: AbortSignal): Promise<GitStatusValue> {
    const result = requireSuccess(
      await this.repo(root, ['status', '--porcelain=v2', '-b', '-z'], signal),
      'git status',
    )
    return parseGitStatus(result.stdout)
  }

  private async repo(root: string, args: readonly string[], signal: AbortSignal): Promise<GitRunResult> {
    const result = await this.run(args, root, signal)
    if (result.exitCode !== 0 && /not a git repository/i.test(result.stderr)) {
      throw new RemoteError('git/not-repository', `"${root}" is not inside a git repository`, { path: root })
    }
    return result
  }
}

/** Host service backing the generated `ctx.remote.git` namespace. */
export class GitController extends TypertRemoteService {
  static inject = ['sessions', 'sandboxPolicy', 'typert']

  static Config: z<GitConfig> = z.object({
    timeoutMs: z.natural().min(1).default(15_000),
    networkTimeoutMs: z.natural().min(1).default(120_000),
    maxDiffBytes: z.natural().min(1).default(512 * 1024),
    maxOutputBytes: z.natural().min(1).default(32 * 1024),
  })

  private readonly commands: GitCommands

  /**
   * @param ctx - Host context carrying Session headers and the sandbox policy root.
   * @param config - Git command timeout and diff cap.
   */
  constructor(ctx: Context, config: GitConfig) {
    super(ctx, 'gitController', { namespace: 'git' })
    this.commands = new GitCommands(
      nativeGitRunner(config.timeoutMs),
      config.maxDiffBytes,
      nativeGitRunner(config.networkTimeoutMs),
      config.maxOutputBytes,
    )
    ctx.inject(['sessions', 'typert'], (scope) => {
      scope.typert.lookups.register('gitWorkspaceScope', {
        parameter: 'gitWorkspaceScope',
        wire: 'gitWorkspaceScopeId',
        resource: 'session',
        hostTypeSymbol: '@deepseek-ai/dsh-api-workspace-controller#GitWorkspaceScope',
        wireTypeSymbol: '@deepseek-ai/dsh-session/types#SessionId',
        resolve: async (sessionId) => {
          const live = scope.sessions.get(sessionId)?.header
          const stored = live === undefined
            ? await scope.get('sessionPersistence')?.stat(sessionId)
            : undefined
          const header = live ?? stored?.header
          if (header === undefined) return undefined
          return {
            sessionId,
            workspaceRoot: header.cwd ?? scope.sandboxPolicy.workspaceRoot,
          }
        },
      })
    })
  }

  /**
   * Read the Session workspace's Git status.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param signal - caller cancellation.
   * @returns the checked-out branch and changed paths.
   */
  @Remote('status')
  status(gitWorkspaceScope: GitWorkspaceScope, signal: AbortSignal): Promise<GitStatusValue> {
    return this.commands.status(gitWorkspaceScope.workspaceRoot, signal)
  }

  /**
   * List the Session workspace's local branches.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param signal - caller cancellation.
   * @returns local branches and the current one.
   */
  @Remote('branches')
  branches(gitWorkspaceScope: GitWorkspaceScope, signal: AbortSignal): Promise<GitBranchesValue> {
    return this.commands.branches(gitWorkspaceScope.workspaceRoot, signal)
  }

  /**
   * Check out one existing local branch in the Session workspace.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param request - branch to check out.
   * @param signal - caller cancellation.
   * @returns the now checked-out branch.
   */
  @Remote('checkout')
  checkout(
    gitWorkspaceScope: GitWorkspaceScope,
    request: GitCheckoutRequest,
    signal: AbortSignal,
  ): Promise<GitCheckoutValue> {
    return this.commands.checkout(gitWorkspaceScope.workspaceRoot, request.branch, signal)
  }

  /**
   * Read one changed path's unified diff.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param request - path and staged side.
   * @param signal - caller cancellation.
   * @returns the patch and its truncation and untracked flags.
   */
  @Remote('diff')
  diff(
    gitWorkspaceScope: GitWorkspaceScope,
    request: GitDiffRequest,
    signal: AbortSignal,
  ): Promise<GitDiffValue> {
    return this.commands.diff(gitWorkspaceScope.workspaceRoot, request, signal)
  }

  /**
   * Stage one changed path.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param request - path to stage.
   * @param signal - caller cancellation.
   * @returns the settled path and verb.
   */
  @Remote('stage')
  stage(
    gitWorkspaceScope: GitWorkspaceScope,
    request: GitPathRequest,
    signal: AbortSignal,
  ): Promise<GitPathValue> {
    return this.commands.stage(gitWorkspaceScope.workspaceRoot, request, signal)
  }

  /**
   * Unstage one path without touching its work-tree content.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param request - path with a staged change.
   * @param signal - caller cancellation.
   * @returns the settled path and verb.
   */
  @Remote('unstage')
  unstage(
    gitWorkspaceScope: GitWorkspaceScope,
    request: GitPathRequest,
    signal: AbortSignal,
  ): Promise<GitPathValue> {
    return this.commands.unstage(gitWorkspaceScope.workspaceRoot, request, signal)
  }

  /**
   * Discard one tracked path's unstaged change.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param request - path with an unstaged change.
   * @param signal - caller cancellation.
   * @returns the settled path and verb.
   */
  @Remote('discard')
  discard(
    gitWorkspaceScope: GitWorkspaceScope,
    request: GitPathRequest,
    signal: AbortSignal,
  ): Promise<GitPathValue> {
    return this.commands.discard(gitWorkspaceScope.workspaceRoot, request, signal)
  }

  /**
   * Commit the staged index.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param request - non-blank commit message.
   * @param signal - caller cancellation.
   * @returns the new commit's full object id.
   */
  @Remote('commit')
  commit(
    gitWorkspaceScope: GitWorkspaceScope,
    request: GitCommitRequest,
    signal: AbortSignal,
  ): Promise<GitCommitValue> {
    return this.commands.commit(gitWorkspaceScope.workspaceRoot, request, signal)
  }

  /**
   * Fetch every remote.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param signal - caller cancellation.
   * @returns git's own summary.
   */
  @Remote('fetch')
  fetch(gitWorkspaceScope: GitWorkspaceScope, signal: AbortSignal): Promise<GitRemoteValue> {
    return this.commands.fetch(gitWorkspaceScope.workspaceRoot, signal)
  }

  /**
   * Fast-forward the current branch from its upstream.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param signal - caller cancellation.
   * @returns git's own summary.
   */
  @Remote('pull')
  pull(gitWorkspaceScope: GitWorkspaceScope, signal: AbortSignal): Promise<GitRemoteValue> {
    return this.commands.pull(gitWorkspaceScope.workspaceRoot, signal)
  }

  /**
   * Push the current branch to its upstream.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param signal - caller cancellation.
   * @returns git's own summary.
   */
  @Remote('push')
  push(gitWorkspaceScope: GitWorkspaceScope, signal: AbortSignal): Promise<GitRemoteValue> {
    return this.commands.push(gitWorkspaceScope.workspaceRoot, signal)
  }

  /**
   * Read the repository's multi-step operation and conflicts.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param signal - caller cancellation.
   * @returns the operation kind and conflict paths.
   */
  @Remote('state')
  state(gitWorkspaceScope: GitWorkspaceScope, signal: AbortSignal): Promise<GitOperationState> {
    return this.commands.state(gitWorkspaceScope.workspaceRoot, signal)
  }

  /**
   * Rebase the current branch onto one local branch.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param request - branch to rebase onto.
   * @param signal - caller cancellation.
   * @returns the resulting operation state.
   */
  @Remote('rebase')
  rebase(
    gitWorkspaceScope: GitWorkspaceScope,
    request: GitRebaseRequest,
    signal: AbortSignal,
  ): Promise<GitOperationValue> {
    return this.commands.rebase(gitWorkspaceScope.workspaceRoot, request, signal)
  }

  /**
   * Cherry-pick one commit onto the current branch.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param request - commit id.
   * @param signal - caller cancellation.
   * @returns the resulting operation state.
   */
  @Remote('cherryPick')
  cherryPick(
    gitWorkspaceScope: GitWorkspaceScope,
    request: GitCherryPickRequest,
    signal: AbortSignal,
  ): Promise<GitOperationValue> {
    return this.commands.cherryPick(gitWorkspaceScope.workspaceRoot, request, signal)
  }

  /**
   * Continue the in-progress rebase or cherry-pick.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param signal - caller cancellation.
   * @returns the resulting operation state.
   */
  @Remote('continueOperation')
  continueOperation(gitWorkspaceScope: GitWorkspaceScope, signal: AbortSignal): Promise<GitOperationValue> {
    return this.commands.continueOperation(gitWorkspaceScope.workspaceRoot, signal)
  }

  /**
   * Abort the in-progress rebase or cherry-pick.
   * @param gitWorkspaceScope - header-derived workspace root for the Session identity on the wire.
   * @param signal - caller cancellation.
   * @returns the restored operation state.
   */
  @Remote('abortOperation')
  abortOperation(gitWorkspaceScope: GitWorkspaceScope, signal: AbortSignal): Promise<GitOperationValue> {
    return this.commands.abortOperation(gitWorkspaceScope.workspaceRoot, signal)
  }
}
