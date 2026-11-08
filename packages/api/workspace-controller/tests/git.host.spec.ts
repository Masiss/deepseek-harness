/** Git Remote contract: porcelain parsing, repository status, and checkout. */
import { existsSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SessionStore, { SESSION_FORMAT_VERSION, SessionId } from '@deepseek-ai/dsh-session'
import TypertRegistry from '@deepseek-ai/dsh-typert-registry'
import { RemoteError } from '@deepseek-ai/dsh-typert-protocol'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  GitCommands,
  GitController,
  nativeGitRunner,
  normalizeGitRefs,
  parseGitBranches,
  parseGitLog,
  parseGitNumstat,
  parseGitStatus,
  type GitRunner,
  type GitRunResult,
} from '../src/git.ts'

const RUNNER = nativeGitRunner(10_000)
const signal = (): AbortSignal => new AbortController().signal

/** Run git synchronously inside a fixture repository. */
function git(cwd: string, ...args: string[]): string {
  return execFileSync('git', [
    '-c', 'user.email=t@example.com', '-c', 'user.name=t', '-c', 'commit.gpgsign=false', ...args,
  ], { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] })
}

/** Run git expecting a nonzero exit, such as the conflict a merge or revert reports. */
function gitTolerating(cwd: string, ...args: string[]): void {
  try {
    git(cwd, ...args)
  } catch (error: unknown) {
    if (!(error instanceof Error)) throw error
  }
}

/** A temporary directory removed after the test. */
async function scratch(prefix: string): Promise<string> {
  return await mkdtemp(join(tmpdir(), prefix))
}

/** Settle a rejected verb into its failure value without an untyped `.catch`. */
async function rejected(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise
    return undefined
  } catch (error: unknown) {
    return error
  }
}

describe('parseGitStatus', () => {
  it('reads the branch header and ordinary changes', () => {
    const output = [
      '# branch.oid abc123',
      '# branch.head main',
      '# branch.upstream origin/main',
      '# branch.ab +1 -0',
      '1 .M N... 100644 100644 100644 aaa bbb src/app.ts',
      '? notes.md',
      '',
    ].join('\0')
    expect(parseGitStatus(output)).toEqual({
      branch: 'main',
      commit: 'abc123',
      detached: false,
      upstream: 'origin/main',
      ahead: 1,
      behind: 0,
      files: [
        { path: 'src/app.ts', status: 'modified', staged: false, worktree: true },
        { path: 'notes.md', status: 'untracked', staged: false, worktree: true },
      ],
    })
  })

  it('reports detached HEAD without inventing a branch', () => {
    const status = parseGitStatus('# branch.oid abc\0# branch.head (detached)\0')
    expect(status).toMatchObject({ branch: null, commit: 'abc', detached: true, upstream: null, ahead: 0, behind: 0 })
  })

  it('reports behind-only divergence', () => {
    const status = parseGitStatus('# branch.head main\0# branch.upstream origin/main\0# branch.ab +0 -3\0')
    expect(status).toMatchObject({ upstream: 'origin/main', ahead: 0, behind: 3 })
  })

  it('reports an unborn branch with no commit', () => {
    const status = parseGitStatus('# branch.oid (initial)\0# branch.head main\0')
    expect(status).toMatchObject({ branch: 'main', commit: null, detached: false })
  })

  it('keeps a rename path, its original path, and skips that record', () => {
    const output = [
      '# branch.head main',
      '2 R. N... 100644 100644 100644 aaa bbb R100 new name.ts',
      'old name.ts',
      '',
    ].join('\0')
    expect(parseGitStatus(output).files).toEqual([
      { path: 'new name.ts', status: 'renamed', staged: true, worktree: false, origPath: 'old name.ts' },
    ])
  })

  it('classifies an unmerged path as a conflict', () => {
    const output = '# branch.head main\0u UU N... 100644 100644 100644 100644 a b c conflict.ts\0'
    expect(parseGitStatus(output).files).toEqual([{ path: 'conflict.ts', status: 'conflict', staged: true, worktree: true }])
  })
})

describe('parseGitBranches', () => {
  it('marks the checked-out branch and keeps tab-separated ids', () => {
    const output = 'feature\tabc\t\nmain\tdef\t*\n'
    expect(parseGitBranches(output, 'main')).toEqual([
      { name: 'feature', commit: 'abc', current: false },
      { name: 'main', commit: 'def', current: true },
    ])
  })
})

describe('GitCommands', () => {
  let root: string

  beforeEach(async () => {
    root = await scratch('dsh-git-controller-')
  })

  afterEach(async () => {
    await rm(root, { recursive: true, force: true })
  })

  it('refuses a directory outside a repository', async () => {
    const failure = await rejected(new GitCommands(RUNNER).status(root, signal()))
    expect(failure).toBeInstanceOf(RemoteError)
    expect(failure).toMatchObject({ code: 'git/not-repository' })
  })

  it('reports an empty repository and its unborn branch', async () => {
    git(root, 'init', '-b', 'main')
    const status = await new GitCommands(RUNNER).status(root, signal())
    expect(status).toMatchObject({ branch: 'main', commit: null, detached: false, files: [] })
  })

  it('reports staged, modified, and untracked paths', async () => {
    git(root, 'init', '-b', 'main')
    await writeFile(join(root, 'staged.ts'), 'export {}\n')
    await writeFile(join(root, 'loose.ts'), 'loose\n')
    git(root, 'add', 'staged.ts')
    const status = await new GitCommands(RUNNER).status(root, signal())
    expect(status.files).toEqual(expect.arrayContaining([
      {
        path: 'staged.ts',
        status: 'added',
        staged: true,
        worktree: false,
        stats: { index: { additions: 1, deletions: 0 } },
      },
      { path: 'loose.ts', status: 'untracked', staged: false, worktree: true },
    ]))
  })

  it('lists local branches and checks one out', async () => {
    git(root, 'init', '-b', 'main')
    await writeFile(join(root, 'a.txt'), 'a\n')
    git(root, 'add', 'a.txt')
    git(root, 'commit', '-m', 'first')
    git(root, 'branch', 'feature')
    const commands = new GitCommands(RUNNER)

    const listed = await commands.branches(root, signal())
    expect(listed.current).toBe('main')
    expect(listed.branches.map(branch => ({ name: branch.name, current: branch.current }))).toEqual([
      { name: 'feature', current: false },
      { name: 'main', current: true },
    ])
    expect(listed.branches.every(branch => branch.commit.length > 0)).toBe(true)

    const checked = await commands.checkout(root, 'feature', signal())
    expect(checked).toEqual({ branch: 'feature' })
    await expect(commands.status(root, signal())).resolves.toMatchObject({ branch: 'feature', detached: false })
  })

  it('refuses an unknown branch without running checkout', async () => {
    git(root, 'init', '-b', 'main')
    const failure = await rejected(new GitCommands(RUNNER).checkout(root, 'missing', signal()))
    expect(failure).toMatchObject({ code: 'git/checkout-refused' })
  })

  it('reports a dirty tree that blocks the switch', async () => {
    git(root, 'init', '-b', 'main')
    await writeFile(join(root, 'a.txt'), 'main\n')
    git(root, 'add', 'a.txt')
    git(root, 'commit', '-m', 'first')
    git(root, 'checkout', '-b', 'feature')
    await writeFile(join(root, 'a.txt'), 'feature\n')
    git(root, 'commit', '-am', 'feature')
    git(root, 'checkout', 'main')
    await writeFile(join(root, 'a.txt'), 'local\n')

    const failure = await rejected(new GitCommands(RUNNER).checkout(root, 'feature', signal()))
    expect(failure).toMatchObject({ code: 'git/checkout-refused' })
  })

  it('reports a detached HEAD', async () => {
    git(root, 'init', '-b', 'main')
    await writeFile(join(root, 'a.txt'), 'a\n')
    git(root, 'add', 'a.txt')
    git(root, 'commit', '-m', 'first')
    git(root, 'checkout', '--detach')
    await expect(new GitCommands(RUNNER).status(root, signal())).resolves.toMatchObject({
      branch: null,
      detached: true,
    })
  })
})

describe('parseGitStatus edge cases', () => {
  it.each([
    ['1 M. N... 100644 100644 100644 a b file', 'modified'],
    ['1 A. N... 100644 100644 100644 a b file', 'added'],
    ['1 .D N... 100644 100644 100644 a b file', 'deleted'],
    ['2 R. N... 100644 100644 100644 a b R100 file', 'renamed'],
    ['2 C. N... 100644 100644 100644 a b C100 file', 'copied'],
    ['1 .T N... 100644 100644 100644 a b file', 'typechange'],
    ['1 .. N... 100644 100644 100644 a b file', 'modified'],
    ['! ignored.txt', 'ignored'],
  ])('maps %s to %s', (record, status) => {
    const output = `# branch.head main\0${record}\0${record.startsWith('2 ') ? 'original\0' : ''}`
    expect(parseGitStatus(output).files[0]?.status).toBe(status)
  })

  it('accepts output without a trailing NUL', () => {
    expect(parseGitStatus('# branch.head main').files).toEqual([])
  })

  it('treats a record with too few fields or an empty tail as truncated', () => {
    expect(() => parseGitStatus('1 M. N...\0')).toThrow('truncated git status record')
    expect(() => parseGitStatus('1 a b c d e f g \0')).toThrow('truncated git status record')
  })

  it('rejects an unrecognized record that would silently drop a change', () => {
    expect(() => parseGitStatus('# branch.head main\0x broken\0')).toThrow('unrecognized git status record')
  })

  it('rejects a truncated record', () => {
    expect(() => parseGitStatus('1 M. N...\0')).toThrow('truncated git status record')
  })

  it('rejects a malformed ahead/behind record', () => {
    expect(() => parseGitStatus('# branch.ab +x -0\0')).toThrow('malformed git status ahead/behind record')
  })

  it('ignores a branch header it does not read', () => {
    expect(parseGitStatus('# branch.unused x\0')).toMatchObject({ branch: null, upstream: null, ahead: 0, behind: 0 })
  })

  it('rejects a rename record without its original path', () => {
    expect(() => parseGitStatus('# branch.head main\u00002 R. N... 100644 100644 100644 a b R100 file\0'))
      .toThrow('truncated git status record')
  })
})

describe('parseGitNumstat', () => {
  it('reads counted paths and renames keyed by the new path', () => {
    const output = ['12\t3\tsrc/app.ts', '0\t0\t', 'old.ts', 'new.ts', '1\t2\ta b.ts', ''].join('\0')
    expect([...parseGitNumstat(output)]).toEqual([
      ['src/app.ts', { additions: 12, deletions: 3 }],
      ['new.ts', { additions: 0, deletions: 0 }],
      ['a b.ts', { additions: 1, deletions: 2 }],
    ])
  })

  it('reports a binary change without counts', () => {
    expect(parseGitNumstat('-\t-\timage.png\0').get('image.png')).toEqual({ additions: null, deletions: null })
  })

  it('accepts output without a trailing NUL', () => {
    expect(parseGitNumstat('12\t3\tsrc/app.ts').get('src/app.ts')).toEqual({ additions: 12, deletions: 3 })
  })

  it('rejects a record without its two count fields', () => {
    expect(() => parseGitNumstat('12\tsrc/app.ts\0')).toThrow('malformed git numstat record')
    expect(() => parseGitNumstat('notenumstat\0')).toThrow('malformed git numstat record: notenumstat')
  })

  it('rejects a count that is neither a number nor a binary marker', () => {
    expect(() => parseGitNumstat('x\t3\tfile\0')).toThrow('malformed git numstat record: x')
  })

  it('rejects a rename record without both names', () => {
    expect(() => parseGitNumstat('0\t0\t\0old.ts\0')).toThrow('truncated git numstat record')
  })
})

describe('parseGitLog', () => {
  /** One log record in the format the verb asks git for. */
  const record = (oid: string, parents: string, refs: string, subject: string): string =>
    `${oid}\0${parents}\0${refs}\0${subject}\0Ada\0ada@example.com\u00002026-11-08T10:00:00+07:00\u001e`

  it('reads newest-first records with parents, refs, and the author instant', () => {
    const output = `\n${record('a'.repeat(40), 'b'.repeat(40), 'HEAD -> main, origin/main', 'second')}\n`
      + `${record('b'.repeat(40), '', 'tag: v1.0.0', 'first')}\n`
    expect(parseGitLog(output)).toEqual([
      {
        oid: 'a'.repeat(40),
        parents: ['b'.repeat(40)],
        refs: ['HEAD', 'main', 'origin/main'],
        subject: 'second',
        authorName: 'Ada',
        authorEmail: 'ada@example.com',
        authorTime: '2026-11-08T10:00:00+07:00',
      },
      {
        oid: 'b'.repeat(40),
        parents: [],
        refs: ['tag:v1.0.0'],
        subject: 'first',
        authorName: 'Ada',
        authorEmail: 'ada@example.com',
        authorTime: '2026-11-08T10:00:00+07:00',
      },
    ])
  })

  it('reads no commits from empty output', () => {
    expect(parseGitLog('')).toEqual([])
  })

  it('rejects a truncated record', () => {
    expect(() => parseGitLog(`${'a'.repeat(40)}\0\0HEAD\0subject\n`)).toThrow('truncated git log record')
  })

  it('rejects a record whose object id is not hexadecimal', () => {
    expect(() => parseGitLog('nope\0\0HEAD\0subject\0Ada\0a@b\u00002026-11-08T10:00:00Z\u001e'))
      .toThrow('malformed git log object id: nope')
  })
})

describe('normalizeGitRefs', () => {
  it('normalizes HEAD arrows, remote symbolic refs, tags, and empty decorations', () => {
    expect(normalizeGitRefs('')).toEqual([])
    expect(normalizeGitRefs('HEAD -> main')).toEqual(['HEAD', 'main'])
    expect(normalizeGitRefs('origin/HEAD -> origin/main')).toEqual(['origin/main'])
    expect(normalizeGitRefs('feature, tag: v1.2.3')).toEqual(['feature', 'tag:v1.2.3'])
  })
})

describe('GitCommands failures', () => {
  it('rejects an unknown branch name before invoking git', async () => {
    const run = vi.fn<GitRunner>()
    const commands = new GitCommands(run)
    await expect(commands.checkout('/repo', '-x', signal())).rejects.toMatchObject({ code: 'git/checkout-refused' })
    await expect(commands.checkout('/repo', 'two words', signal())).rejects.toMatchObject({ code: 'git/checkout-refused' })
    expect(run).not.toHaveBeenCalled()
  })

  it('maps an unexpected nonzero status exit to git/command-failed', async () => {
    const commands = new GitCommands(async () => ({ stdout: '', stderr: 'boom', exitCode: 2 }))
    await expect(commands.status('/repo', signal())).rejects.toMatchObject({ code: 'git/command-failed' })
  })

  it('names the exit code when git reports no stderr', async () => {
    const commands = new GitCommands(async () => ({ stdout: '', stderr: '', exitCode: 3 }))
    await expect(commands.status('/repo', signal())).rejects.toThrow('git status failed: exit 3')
  })

  it('reports a checkout failure that carries no stderr', async () => {
    const commands = new GitCommands(async (args) => {
      if (args[0] === 'status') return { stdout: '', stderr: '', exitCode: 0 }
      if (args[0] === 'for-each-ref') return { stdout: 'main\tabc\t*\n', stderr: '', exitCode: 0 }
      return { stdout: '', stderr: '', exitCode: 1 }
    })
    await expect(commands.checkout('/repo', 'main', signal())).rejects.toThrow('failed: exit 1')
  })

  it('carries the executable stderr and exit code of a failing child', async () => {
    const commands = new GitCommands(nativeGitRunner(2_000, '/bin/false'))
    await expect(commands.status('/repo', signal())).rejects.toMatchObject({ code: 'git/command-failed' })
  })

  it('maps a missing executable to git/unavailable', async () => {
    const commands = new GitCommands(nativeGitRunner(2_000, 'dsh-missing-git-executable'))
    await expect(commands.status('/repo', signal())).rejects.toMatchObject({ code: 'git/unavailable' })
  })

  it('maps an unspawnable file to git/command-failed with exit 1', async () => {
    const directory = await scratch('dsh-git-not-executable-')
    const file = join(directory, 'git')
    try {
      await writeFile(file, 'not executable\n')
      const commands = new GitCommands(nativeGitRunner(2_000, file))
      await expect(commands.status('/repo', signal())).rejects.toMatchObject({ code: 'git/command-failed' })
    } finally {
      await rm(directory, { recursive: true, force: true })
    }
  })

  it('maps a pre-aborted signal to gateway/cancelled', async () => {
    const controller = new AbortController()
    controller.abort()
    const commands = new GitCommands(nativeGitRunner(2_000))
    await expect(commands.status('/repo', controller.signal)).rejects.toMatchObject({ code: 'gateway/cancelled' })
  })

  it('rejects a malformed branch record', () => {
    expect(() => parseGitBranches('only-a-name\n', null)).toThrow('malformed git branch record')
  })
})

describe('GitController Session scope', () => {
  it('resolves live and stored Session headers and leaves with its plugin', async () => {
    const root = await scratch('dsh-git-scope-')
    const remote = await scratch('dsh-git-scope-remote-')
    const liveId = SessionId('live-git-session')
    const storedId = SessionId('stored-git-session')
    const missingId = SessionId('missing-git-session')
    const fallbackId = SessionId('stored-without-cwd')
    const stat = vi.fn(async (id: SessionId) => {
      if (id === storedId) {
        return { header: { version: SESSION_FORMAT_VERSION, id, createdAt: 1, isSeeded: false, origin: 'subagent' as const, cwd: root } }
      }
      if (id === fallbackId) {
        return { header: { version: SESSION_FORMAT_VERSION, id, createdAt: 1, isSeeded: false, origin: 'subagent' as const } }
      }
      return undefined
    })
    const ctx = new Context()
    ctx.provide('sandboxPolicy', { workspaceRoot: root } as never)
    ctx.provide('sessionPersistence', { stat } as never)
    const sessions = await ctx.plugin(SessionStore)
    const typert = await ctx.plugin(TypertRegistry)
    const plugin = await ctx.plugin(GitController, {
      // Short local timeout so a hung child fails the test instead of the suite.
      timeoutMs: 5_000,
      networkTimeoutMs: 120_000,
      maxDiffBytes: 512 * 1024,
      maxOutputBytes: 32 * 1024,
    })
    const controller = ctx.gitController

    try {
      git(root, 'init', '-b', 'main')
      await writeFile(join(root, 'a.txt'), 'a\n')
      git(root, 'add', 'a.txt')
      git(root, 'commit', '-m', 'first')
      git(root, 'branch', 'feature')
      git(remote, 'init', '--bare', '-b', 'main')
      git(root, 'remote', 'add', 'origin', remote)
      git(root, 'push', '-u', 'origin', 'main')
      ctx.sessions.create(liveId, { meta: { cwd: root, origin: 'subagent' } })

      const lookup = ctx.typert.lookups.get('gitWorkspaceScope')
      expect(lookup).toMatchObject({
        parameter: 'gitWorkspaceScope',
        wire: 'gitWorkspaceScopeId',
        resource: 'session',
        hostTypeSymbol: '@deepseek-ai/dsh-api-workspace-controller#GitWorkspaceScope',
        wireTypeSymbol: '@deepseek-ai/dsh-session/types#SessionId',
      })
      if (lookup === undefined) throw new Error('gitWorkspaceScope lookup did not register')
      await expect(lookup.resolve(liveId)).resolves.toEqual({ sessionId: liveId, workspaceRoot: root })
      await expect(lookup.resolve(storedId)).resolves.toEqual({ sessionId: storedId, workspaceRoot: root })
      await expect(lookup.resolve(fallbackId)).resolves.toEqual({ sessionId: fallbackId, workspaceRoot: root })
      await expect(lookup.resolve(missingId)).resolves.toBeUndefined()

      const scope = { sessionId: liveId, workspaceRoot: root }
      await expect(controller.status(scope, signal())).resolves.toMatchObject({ branch: 'main', detached: false })
      await expect(controller.branches(scope, signal())).resolves.toMatchObject({ current: 'main' })
      await expect(controller.checkout(scope, { branch: 'feature' }, signal())).resolves.toEqual({ branch: 'feature' })
      await writeFile(join(root, 'a.txt'), 'changed\n')
      await expect(controller.diff(scope, { path: 'a.txt', staged: false }, signal()))
        .resolves.toMatchObject({ path: 'a.txt', staged: false })
      await expect(controller.stage(scope, { path: 'a.txt' }, signal()))
        .resolves.toEqual({ path: 'a.txt', operation: 'stage' })
      await expect(controller.unstage(scope, { path: 'a.txt' }, signal()))
        .resolves.toEqual({ path: 'a.txt', operation: 'unstage' })
      await expect(controller.discard(scope, { path: 'a.txt' }, signal()))
        .resolves.toEqual({ path: 'a.txt', operation: 'discard' })
      await writeFile(join(root, 'a.txt'), 'committed\n')
      await controller.stage(scope, { path: 'a.txt' }, signal())
      const committed = await controller.commit(scope, { message: 'via controller' }, signal())
      expect(committed.commit).toMatch(/^[0-9a-f]{40}$/)

      await writeFile(join(root, 'a.txt'), 'hunk\n')
      const patch = (await controller.diff(scope, { path: 'a.txt', staged: false }, signal())).patch
      await expect(controller.apply(scope, { path: 'a.txt', patch, target: 'index' }, signal()))
        .resolves.toEqual({ path: 'a.txt', operation: 'stage' })
      await expect(controller.apply(scope, { path: 'a.txt', patch, target: 'index', reverse: true }, signal()))
        .resolves.toEqual({ path: 'a.txt', operation: 'unstage' })
      await expect(controller.discard(scope, { path: 'a.txt' }, signal()))
        .resolves.toEqual({ path: 'a.txt', operation: 'discard' })
      const history = await controller.log(scope, { limit: 2 }, signal())
      expect(history.commits[0]?.oid).toBe(committed.commit)
      await expect(controller.show(scope, { commit: committed.commit }, signal()))
        .resolves.toMatchObject({ commit: committed.commit, truncated: false })

      await expect(controller.state(scope, signal())).resolves.toEqual({ operation: 'none', conflicts: [] })
      await expect(controller.rebase(scope, { branch: 'main' }, signal()))
        .resolves.toMatchObject({ state: { operation: 'none' } })
      git(root, 'checkout', 'main')
      await expect(controller.cherryPick(scope, { commit: committed.commit }, signal()))
        .resolves.toMatchObject({ state: { operation: 'none' } })
      await expect(controller.merge(scope, { branch: 'feature' }, signal()))
        .resolves.toMatchObject({ state: { operation: 'none' } })
      await writeFile(join(root, 'b.txt'), 'to revert\n')
      await controller.stage(scope, { path: 'b.txt' }, signal())
      const reverted = await controller.commit(scope, { message: 'to revert' }, signal())
      await expect(controller.revert(scope, { commit: reverted.commit }, signal()))
        .resolves.toMatchObject({ state: { operation: 'none' } })
      await expect(controller.fetch(scope, signal())).resolves.toMatchObject({ operation: 'fetch' })
      await expect(controller.pull(scope, signal())).resolves.toMatchObject({ operation: 'pull' })
      await expect(controller.push(scope, {}, signal())).resolves.toMatchObject({ operation: 'push' })
      // The wire field is optional, so an absent request reaches the verb as undefined.
      await expect(controller.push(scope, undefined, signal())).resolves.toMatchObject({ operation: 'push' })
      await expect(rejected(controller.continueOperation(scope, signal())))
        .resolves.toMatchObject({ code: 'git/operation-refused' })
      await expect(rejected(controller.abortOperation(scope, signal())))
        .resolves.toMatchObject({ code: 'git/operation-refused' })

      await plugin.dispose()
      expect(ctx.typert.lookups.get('gitWorkspaceScope')).toBeUndefined()
    } finally {
      await plugin.dispose()
      await sessions.dispose()
      await typert.dispose()
      await rm(root, { recursive: true, force: true })
      await rm(remote, { recursive: true, force: true })
    }
  })
})

describe('GitCommands diff and index verbs', () => {
  let root: string

  beforeEach(async () => {
    root = await scratch('dsh-git-index-')
    git(root, 'init', '-b', 'main')
    await writeFile(join(root, 'a.txt'), 'one\n')
    git(root, 'add', 'a.txt')
    git(root, 'commit', '-m', 'first')
  })

  afterEach(async () => {
    await rm(root, { recursive: true, force: true })
  })

  it('reads the unstaged diff, stages it, reads the staged diff, and unstages it', async () => {
    await writeFile(join(root, 'a.txt'), 'two\n')
    const commands = new GitCommands(RUNNER)

    const unstaged = await commands.diff(root, { path: 'a.txt', staged: false }, signal())
    expect(unstaged).toMatchObject({ path: 'a.txt', staged: false, truncated: false, untracked: false })
    expect(unstaged.patch).toContain('-one')
    expect(unstaged.patch).toContain('+two')

    await expect(commands.stage(root, { path: 'a.txt' }, signal())).resolves.toEqual({ path: 'a.txt', operation: 'stage' })
    expect((await commands.status(root, signal())).files[0]).toMatchObject({ path: 'a.txt', staged: true, worktree: false })

    const staged = await commands.diff(root, { path: 'a.txt', staged: true }, signal())
    expect(staged.patch).toContain('+two')

    await expect(commands.unstage(root, { path: 'a.txt' }, signal())).resolves.toEqual({ path: 'a.txt', operation: 'unstage' })
    expect((await commands.status(root, signal())).files[0]).toMatchObject({ staged: false, worktree: true })
  })

  it('renders an untracked file through --no-index', async () => {
    await writeFile(join(root, 'loose.txt'), 'loose\n')
    const diff = await new GitCommands(RUNNER).diff(root, { path: 'loose.txt', staged: false }, signal())
    expect(diff).toMatchObject({ untracked: true, truncated: false })
    expect(diff.patch).toContain('+loose')
  })

  it('discards an unstaged modification without touching the index', async () => {
    await writeFile(join(root, 'a.txt'), 'local\n')
    await expect(new GitCommands(RUNNER).discard(root, { path: 'a.txt' }, signal()))
      .resolves.toEqual({ path: 'a.txt', operation: 'discard' })
    await expect(readFile(join(root, 'a.txt'), 'utf8')).resolves.toBe('one\n')
  })

  it('applies a work-tree patch to the index and unstages or discards it backwards', async () => {
    await writeFile(join(root, 'a.txt'), 'two\n')
    const commands = new GitCommands(RUNNER)
    const patch = (await commands.diff(root, { path: 'a.txt', staged: false }, signal())).patch

    await expect(commands.apply(root, { path: 'a.txt', patch, target: 'index' }, signal()))
      .resolves.toEqual({ path: 'a.txt', operation: 'stage' })
    expect((await commands.status(root, signal())).files[0]).toMatchObject({ staged: true, worktree: false })

    await expect(commands.apply(root, { path: 'a.txt', patch, target: 'index', reverse: true }, signal()))
      .resolves.toEqual({ path: 'a.txt', operation: 'unstage' })
    expect((await commands.status(root, signal())).files[0]).toMatchObject({ staged: false, worktree: true })

    await expect(commands.apply(root, { path: 'a.txt', patch, target: 'worktree', reverse: true }, signal()))
      .resolves.toEqual({ path: 'a.txt', operation: 'discard' })
    await expect(readFile(join(root, 'a.txt'), 'utf8')).resolves.toBe('one\n')
  })

  it('asks git for unquoted paths so a patch header names the caller path', async () => {
    const calls: string[][] = []
    const run: GitRunner = async (args) => {
      calls.push([...args])
      return args[0] === 'status'
        ? { stdout: '1 .M N... 100644 100644 100644 aaa bbb a.txt\0', stderr: '', exitCode: 0 }
        : { stdout: '', stderr: '', exitCode: 0 }
    }
    await new GitCommands(run).diff('/repo', { path: 'a.txt', staged: false }, signal())
    expect(calls[1]).toEqual([
      '-c', 'core.quotePath=false', 'diff', '--no-color', '--no-ext-diff', '--unified=3', '--', 'a.txt',
    ])
  })

  it('applies a patch for a non-ASCII path and refuses it under another path', async () => {
    const name = 'é.txt'
    await writeFile(join(root, name), 'one\n')
    git(root, 'add', name)
    git(root, 'commit', '-m', 'unicode')
    await writeFile(join(root, name), 'two\n')
    const commands = new GitCommands(RUNNER)
    const patch = (await commands.diff(root, { path: name, staged: false }, signal())).patch
    expect(patch.startsWith(`diff --git a/${name} b/${name}`)).toBe(true)

    await expect(commands.apply(root, { path: name, patch, target: 'index' }, signal()))
      .resolves.toEqual({ path: name, operation: 'stage' })
    await expect(commands.apply(root, { path: name, patch, target: 'index', reverse: true }, signal()))
      .resolves.toEqual({ path: name, operation: 'unstage' })
    await expect(commands.apply(root, { path: 'a.txt', patch, target: 'index' }, signal()))
      .rejects.toMatchObject({ code: 'git/apply-refused' })
  })

  it('refuses a patch that no longer applies and leaves the index untouched', async () => {
    await writeFile(join(root, 'a.txt'), 'two\n')
    const commands = new GitCommands(RUNNER)
    const patch = (await commands.diff(root, { path: 'a.txt', staged: false }, signal())).patch
    await commands.apply(root, { path: 'a.txt', patch, target: 'index' }, signal())
    await expect(commands.apply(root, { path: 'a.txt', patch, target: 'index' }, signal()))
      .rejects.toMatchObject({ code: 'git/apply-refused' })
  })

  it('refuses a discard whose reviewed status no longer matches', async () => {
    await writeFile(join(root, 'a.txt'), 'local\n')
    const commands = new GitCommands(RUNNER)
    await expect(
      commands.discard(root, { path: 'a.txt', expectedStatus: 'deleted' }, signal()),
    ).rejects.toMatchObject({ code: 'git/path-refused' })
    await expect(readFile(join(root, 'a.txt'), 'utf8')).resolves.toBe('local\n')
    await expect(
      commands.discard(root, { path: 'a.txt', expectedStatus: 'modified' }, signal()),
    ).resolves.toEqual({ path: 'a.txt', operation: 'discard' })
  })

  it('commits the staged index', async () => {
    await writeFile(join(root, 'a.txt'), 'two\n')
    const commands = new GitCommands(RUNNER)
    await commands.stage(root, { path: 'a.txt' }, signal())
    const committed = await commands.commit(root, { message: 'second' }, signal())
    expect(committed.commit).toMatch(/^[0-9a-f]{40}$/)
    await expect(commands.status(root, signal())).resolves.toMatchObject({ files: [] })
  })

  it('refuses an empty message and a git commit that finds nothing staged', async () => {
    await expect(new GitCommands(RUNNER).commit(root, { message: '   ' }, signal()))
      .rejects.toMatchObject({ code: 'git/commit-refused' })
    await expect(new GitCommands(RUNNER).commit(root, { message: 'empty' }, signal()))
      .rejects.toMatchObject({ code: 'git/commit-refused' })
  })

  it('refuses paths outside the repository, unknown paths, and unsafe discards', async () => {
    const commands = new GitCommands(RUNNER)
    await writeFile(join(root, 'loose.txt'), 'loose\n')
    await writeFile(join(root, 'a.txt'), 'local\n')
    for (const path of ['../escape', '/absolute', 'C:\\absolute', 'a/../b', '']) {
      await expect(commands.stage(root, { path }, signal())).rejects.toMatchObject({ code: 'git/path-refused' })
      await expect(commands.diff(root, { path, staged: false }, signal())).rejects.toMatchObject({ code: 'git/path-refused' })
    }
    await expect(commands.stage(root, { path: 'missing.txt' }, signal())).rejects.toMatchObject({ code: 'git/path-refused' })
    await expect(commands.discard(root, { path: 'loose.txt' }, signal())).rejects.toMatchObject({ code: 'git/path-refused' })
  })

  it('refuses to unstage a path with no staged change', async () => {
    await writeFile(join(root, 'a.txt'), 'local\n')
    await expect(new GitCommands(RUNNER).unstage(root, { path: 'a.txt' }, signal()))
      .rejects.toMatchObject({ code: 'git/path-refused' })
  })

  it('refuses to discard when the index holds the only change', async () => {
    await writeFile(join(root, 'a.txt'), 'two\n')
    const commands = new GitCommands(RUNNER)
    await commands.stage(root, { path: 'a.txt' }, signal())
    await expect(commands.discard(root, { path: 'a.txt' }, signal()))
      .rejects.toMatchObject({ code: 'git/path-refused' })
  })

  it('flags a patch that exceeds the configured byte cap', async () => {
    await writeFile(join(root, 'a.txt'), 'a much longer second line\n')
    const diff = await new GitCommands(RUNNER, 20).diff(root, { path: 'a.txt', staged: false }, signal())
    expect(diff.truncated).toBe(true)
    expect(Buffer.byteLength(diff.patch, 'utf8')).toBeLessThanOrEqual(20)
  })

  it('reports a no-index diff failure other than the expected difference exit', async () => {
    const commands = new GitCommands(async args => args.includes('--no-index')
      ? { stdout: '', stderr: 'broken', exitCode: 2 }
      : { stdout: '? loose.txt\0', stderr: '', exitCode: 0 })
    await expect(commands.diff(root, { path: 'loose.txt', staged: false }, signal()))
      .rejects.toMatchObject({ code: 'git/command-failed' })
  })

  it('reports a commit failure that carries no stderr', async () => {
    const commands = new GitCommands(async args => args[0] === 'commit'
      ? { stdout: '', stderr: '', exitCode: 1 }
      : { stdout: '', stderr: '', exitCode: 0 })
    await expect(commands.commit(root, { message: 'x' }, signal())).rejects.toThrow('failed: exit 1')
  })
})

describe('GitCommands status details', () => {
  it('reports upstream divergence, rename sources, and per-side line totals', async () => {
    const calls: string[][] = []
    const run: GitRunner = async (args) => {
      calls.push([...args])
      if (args[0] === 'status') {
        return {
          stdout: [
            '# branch.oid abc123',
            '# branch.head main',
            '# branch.upstream origin/main',
            '# branch.ab +2 -3',
            '2 R. N... 100644 100644 100644 aaa bbb R100 new.ts',
            'old.ts',
            '1 .M N... 100644 100644 100644 aaa bbb plain.ts',
            '',
          ].join('\0'),
          stderr: '',
          exitCode: 0,
        }
      }
      if (args.includes('--cached')) return { stdout: '2\t0\tboth.ts\0', stderr: '', exitCode: 0 }
      return { stdout: '4\t1\tplain.ts\0', stderr: '', exitCode: 0 }
    }

    const status = await new GitCommands(run).status('/repo', signal())
    expect(status).toMatchObject({ branch: 'main', upstream: 'origin/main', ahead: 2, behind: 3 })
    expect(status.files).toEqual([
      { path: 'new.ts', status: 'renamed', staged: true, worktree: false, origPath: 'old.ts' },
      {
        path: 'plain.ts',
        status: 'modified',
        staged: false,
        worktree: true,
        stats: { worktree: { additions: 4, deletions: 1 } },
      },
    ])
    expect(calls[1]).toContain('--cached')
    expect(calls[2]).not.toContain('--cached')

    // One path can carry an index and a work-tree change at once, which keeps both sides.
    const both: GitRunner = async (args) => {
      if (args[0] === 'status') {
        return { stdout: '1 MM N... 100644 100644 100644 aaa bbb both.ts\0', stderr: '', exitCode: 0 }
      }
      return args.includes('--cached')
        ? { stdout: '2\t0\tboth.ts\0', stderr: '', exitCode: 0 }
        : { stdout: '1\t1\tboth.ts\0', stderr: '', exitCode: 0 }
    }
    expect((await new GitCommands(both).status('/repo', signal())).files[0]).toEqual({
      path: 'both.ts',
      status: 'modified',
      staged: true,
      worktree: true,
      stats: { index: { additions: 2, deletions: 0 }, worktree: { additions: 1, deletions: 1 } },
    })
  })

  it('omits the side a binary change reports and maps a failed numstat to git/command-failed', async () => {
    const binary: GitRunner = async args => args[0] === 'status'
      ? { stdout: '1 .M N... 100644 100644 100644 aaa bbb image.png\0', stderr: '', exitCode: 0 }
      : { stdout: '-\t-\timage.png\0', stderr: '', exitCode: 0 }
    await expect(new GitCommands(binary).status('/repo', signal())).resolves.toMatchObject({
      files: [{ path: 'image.png', status: 'modified', staged: false, worktree: true }],
    })

    const broken: GitRunner = async args => args[0] === 'status'
      ? { stdout: '1 .M N... 100644 100644 100644 aaa bbb a.ts\0', stderr: '', exitCode: 0 }
      : { stdout: '', stderr: 'boom', exitCode: 2 }
    await expect(new GitCommands(broken).status('/repo', signal()))
      .rejects.toMatchObject({ code: 'git/command-failed' })
  })
})

describe('GitCommands patch application', () => {
  /** One section that changes `src/app.ts` in a single hunk. */
  const PATCH = [
    'diff --git a/src/app.ts b/src/app.ts',
    'index 1111111..2222222 100644',
    '--- a/src/app.ts',
    '+++ b/src/app.ts',
    '@@ -1 +1 @@',
    '-one',
    '+two',
    '',
  ].join('\n')

  /** Porcelain status naming one modified path. */
  const CHANGED = ['# branch.head main', '1 .M N... 100644 100644 100644 aaa bbb src/app.ts', ''].join('\0')

  /** A runner recording argv and answering `git status` plus every other command. */
  function recordingRunner(
    options: { status?: string; answer?: (call: number) => GitRunResult } = {},
  ): { run: GitRunner; calls: string[][] } {
    const calls: string[][] = []
    const run: GitRunner = async (args) => {
      calls.push([...args])
      if (args[0] === 'status') return { stdout: options.status ?? CHANGED, stderr: '', exitCode: 0 }
      return options.answer?.(calls.length) ?? { stdout: '', stderr: '', exitCode: 0 }
    }
    return { run, calls }
  }

  it('checks the patch, applies it to the index, and removes the temporary file', async () => {
    const { run, calls } = recordingRunner()
    await expect(new GitCommands(run).apply('/repo', { path: 'src/app.ts', patch: PATCH, target: 'index' }, signal()))
      .resolves.toEqual({ path: 'src/app.ts', operation: 'stage' })
    const applied = calls.filter(args => args[0] === 'apply')
    expect(applied).toHaveLength(2)
    expect(applied[0]?.slice(0, 3)).toEqual(['apply', '--cached', '--check'])
    expect(applied[1]?.slice(0, 2)).toEqual(['apply', '--cached'])
    const file = applied[0]?.at(-1) ?? ''
    expect(file.endsWith('patch.diff')).toBe(true)
    expect(existsSync(file)).toBe(false)
  })

  it.each([
    ['stage', { target: 'index' }],
    ['unstage', { target: 'index', reverse: true }],
    ['discard', { target: 'worktree', reverse: true }],
    ['apply', { target: 'worktree' }],
  ] as const)('settles a %s request', async (operation, selection) => {
    const { run } = recordingRunner()
    await expect(new GitCommands(run).apply('/repo', { path: 'src/app.ts', patch: PATCH, ...selection }, signal()))
      .resolves.toEqual({ path: 'src/app.ts', operation })
  })

  it('refuses a patch whose check fails with git stderr', async () => {
    const { run, calls } = recordingRunner({ answer: () => ({ stdout: '', stderr: 'patch does not apply', exitCode: 1 }) })
    const commands = new GitCommands(run)
    const refused = commands.apply('/repo', { path: 'src/app.ts', patch: PATCH, target: 'index' }, signal())
    await expect(refused).rejects.toMatchObject({ code: 'git/apply-refused' })
    await expect(refused).rejects.toThrow('git apply check refused the patch for "src/app.ts": patch does not apply')
    expect(calls.filter(args => args[0] === 'apply')).toHaveLength(1)
  })

  it('names the exit code when a refused patch carries no stderr', async () => {
    const { run } = recordingRunner({ answer: () => ({ stdout: '', stderr: '', exitCode: 3 }) })
    await expect(new GitCommands(run).apply('/repo', { path: 'src/app.ts', patch: PATCH, target: 'index' }, signal()))
      .rejects.toThrow('refused the patch for "src/app.ts": exit 3')
  })

  it('reports a failed apply after the check passed', async () => {
    const { run } = recordingRunner({
      answer: call => call <= 2
        ? { stdout: '', stderr: '', exitCode: 0 }
        : { stdout: '', stderr: 'write failed', exitCode: 1 },
    })
    await expect(new GitCommands(run).apply('/repo', { path: 'src/app.ts', patch: PATCH, target: 'worktree' }, signal()))
      .rejects.toThrow('git apply apply refused the patch for "src/app.ts": write failed')
  })

  it.each([
    ['another path', PATCH.replaceAll('src/app.ts', 'src/other.ts'), 'another path or a rename'],
    ['no section', 'not a patch\n', 'no diff --git header'],
    ['a foreign --- header', PATCH.replace('--- a/src/app.ts', '--- a/src/other.ts'), 'its --- header'],
    ['a foreign +++ header', PATCH.replace('+++ b/src/app.ts', '+++ b/src/other.ts'), 'its +++ header'],
  ])('refuses a patch that names %s', async (_case, patch, detail) => {
    const run = vi.fn<GitRunner>()
    await expect(new GitCommands(run).apply('/repo', { path: 'src/app.ts', patch, target: 'index' }, signal()))
      .rejects.toThrow(detail)
    expect(run).not.toHaveBeenCalled()
  })

  it('refuses an empty patch and a patch past the byte cap', async () => {
    const { run } = recordingRunner()
    await expect(new GitCommands(run).apply('/repo', { path: 'src/app.ts', patch: ' \n', target: 'index' }, signal()))
      .rejects.toThrow('the patch is empty')
    await expect(new GitCommands(run, 16).apply('/repo', { path: 'src/app.ts', patch: PATCH, target: 'index' }, signal()))
      .rejects.toThrow('the patch exceeds 16 bytes')
  })

  it('refuses a stale reviewed status and an ineligible path', async () => {
    const { run } = recordingRunner()
    const commands = new GitCommands(run)
    await expect(commands.apply('/repo', {
      path: 'src/app.ts', patch: PATCH, target: 'index', expectedStatus: 'deleted',
    }, signal())).rejects.toThrow('changed from deleted to modified since review')
    const loose = recordingRunner({ status: '? src/app.ts\0' })
    await expect(new GitCommands(loose.run).apply('/repo', { path: 'src/app.ts', patch: PATCH, target: 'index' }, signal()))
      .rejects.toMatchObject({ code: 'git/path-refused' })
  })

  it('refuses a path outside the repository before invoking git', async () => {
    const run = vi.fn<GitRunner>()
    await expect(new GitCommands(run).apply('/repo', { path: '../x', patch: PATCH, target: 'index' }, signal()))
      .rejects.toMatchObject({ code: 'git/path-refused' })
    expect(run).not.toHaveBeenCalled()
  })
})

describe('GitCommands commit options', () => {
  /** A runner recording argv and answering the commit and head lookup. */
  function recordingRunner(): { run: GitRunner; calls: string[][] } {
    const calls: string[][] = []
    const run: GitRunner = async (args) => {
      calls.push([...args])
      return args[0] === 'rev-parse'
        ? { stdout: 'abc123\n', stderr: '', exitCode: 0 }
        : { stdout: '', stderr: '', exitCode: 0 }
    }
    return { run, calls }
  }

  it('passes amend and sign-off to git and returns the new head', async () => {
    const { run, calls } = recordingRunner()
    await expect(new GitCommands(run).commit('/repo', { message: ' fix\n', amend: true, signOff: true }, signal()))
      .resolves.toEqual({ commit: 'abc123' })
    expect(calls[0]).toEqual(['commit', '--amend', '--signoff', '-m', 'fix'])
  })

  it('commits without options by default', async () => {
    const { run, calls } = recordingRunner()
    await new GitCommands(run).commit('/repo', { message: 'plain' }, signal())
    expect(calls[0]).toEqual(['commit', '-m', 'plain'])
  })

  it('signs the commit off when git records the trailer', async () => {
    const directory = await scratch('dsh-git-signoff-')
    try {
      git(directory, 'init', '-b', 'main')
      git(directory, 'config', 'user.email', 't@example.com')
      git(directory, 'config', 'user.name', 't')
      await writeFile(join(directory, 'a.txt'), 'a\n')
      const commands = new GitCommands(RUNNER)
      await commands.stage(directory, { path: 'a.txt' }, signal())
      const signed = await commands.commit(directory, { message: 'signed', signOff: true }, signal())
      expect(signed.commit).toMatch(/^[0-9a-f]{40}$/)
      expect(git(directory, 'log', '-1', '--format=%B')).toContain('Signed-off-by: t <t@example.com>')
    } finally {
      await rm(directory, { recursive: true, force: true })
    }
  })

  it('amends the last commit and refuses to amend an unborn HEAD', async () => {
    const directory = await scratch('dsh-git-amend-')
    try {
      git(directory, 'init', '-b', 'main')
      const commands = new GitCommands(RUNNER)
      await expect(commands.commit(directory, { message: 'nothing yet', amend: true }, signal()))
        .rejects.toMatchObject({ code: 'git/commit-refused' })
      await writeFile(join(directory, 'a.txt'), 'a\n')
      await commands.stage(directory, { path: 'a.txt' }, signal())
      await commands.commit(directory, { message: 'first' }, signal())
      const amended = await commands.commit(directory, { message: 'amended', amend: true }, signal())
      expect(amended.commit).toMatch(/^[0-9a-f]{40}$/)
      expect(git(directory, 'log', '-1', '--format=%s')).toBe('amended\n')
      expect(git(directory, 'rev-list', '--count', 'HEAD')).toBe('1\n')
    } finally {
      await rm(directory, { recursive: true, force: true })
    }
  })
})

describe('GitCommands publish', () => {
  /** A runner answering status and the remote listing. */
  function localRunner(status: string, remotes: string): GitRunner {
    return async args => args[0] === 'status'
      ? { stdout: status, stderr: '', exitCode: 0 }
      : { stdout: remotes, stderr: '', exitCode: 0 }
  }

  it('pushes the current branch and records origin as its upstream', async () => {
    const pushed: string[][] = []
    const networkRun: GitRunner = async (args) => {
      pushed.push([...args])
      return { stdout: '', stderr: 'branch set up', exitCode: 0 }
    }
    const commands = new GitCommands(localRunner('# branch.head main\0', 'origin\nupstream\n'), 1024, networkRun, 1024)
    await expect(commands.push('/repo', { setUpstream: true }, signal()))
      .resolves.toEqual({ operation: 'push', summary: 'branch set up', truncated: false })
    expect(pushed[0]).toEqual(['push', '--set-upstream', 'origin', 'main'])
  })

  it('pushes without publishing when the request is omitted', async () => {
    const pushed: string[][] = []
    const networkRun: GitRunner = async (args) => {
      pushed.push([...args])
      return { stdout: '', stderr: 'up to date', exitCode: 0 }
    }
    const local = vi.fn<GitRunner>()
    const commands = new GitCommands(local, 1024, networkRun, 1024)
    await expect(commands.push('/repo', undefined, signal()))
      .resolves.toEqual({ operation: 'push', summary: 'up to date', truncated: false })
    expect(pushed).toEqual([['push']])
    // A plain push reads no status and no remote list.
    expect(local).not.toHaveBeenCalled()
  })

  it('refuses to publish a detached HEAD', async () => {
    const run = vi.fn<GitRunner>()
    const commands = new GitCommands(localRunner('# branch.head (detached)\0', 'origin\n'), 1024, run, 1024)
    await expect(commands.push('/repo', { setUpstream: true }, signal()))
      .rejects.toThrow('publishing needs a checked-out branch')
    expect(run).not.toHaveBeenCalled()
  })

  it('refuses to publish without an origin remote', async () => {
    const run = vi.fn<GitRunner>()
    const commands = new GitCommands(localRunner('# branch.head main\0', 'upstream\n'), 1024, run, 1024)
    await expect(commands.push('/repo', { setUpstream: true }, signal()))
      .rejects.toMatchObject({ code: 'git/remote-failed' })
    expect(run).not.toHaveBeenCalled()
  })
})

describe('GitCommands log', () => {
  /** One log record in the format the verb asks git for. */
  function record(oid: string, subject: string): string {
    return `${oid}\0\0HEAD -> main\0${subject}\0Ada\0ada@example.com\u00002026-11-08T10:00:00+07:00\u001e`
  }

  const FIRST = record('a'.repeat(40), 'first')
  const SECOND = record('b'.repeat(40), 'second')

  /** A runner recording argv and answering `git log` with one fixed page. */
  function logRunner(output: string): { run: GitRunner; calls: string[][] } {
    const calls: string[][] = []
    const run: GitRunner = async (args) => {
      calls.push([...args])
      return { stdout: output, stderr: '', exitCode: 0 }
    }
    return { run, calls }
  }

  it('reads a capped page and defaults the page size', async () => {
    const { run, calls } = logRunner(`\n${FIRST}\n${SECOND}\n`)
    const commands = new GitCommands(run)
    const page = await commands.log('/repo', {}, signal())
    expect(page.truncated).toBe(false)
    expect(page.commits.map(commit => commit.subject)).toEqual(['first', 'second'])
    expect(calls[0]?.slice(0, 4)).toEqual(['log', '--no-color', '--max-count=50', '--skip=0'])
    await commands.log('/repo', { limit: 500, skip: 5 }, signal())
    expect(calls[1]?.slice(0, 4)).toEqual(['log', '--no-color', '--max-count=200', '--skip=5'])
  })

  it('drops commits past the configured byte cap', async () => {
    const { run } = logRunner(`\n${FIRST}\n${SECOND}\n`)
    await expect(new GitCommands(run, 1024, run, 1).log('/repo', {}, signal()))
      .resolves.toEqual({ commits: [], truncated: true })
  })

  it.each([
    [{ limit: 0 }, 'limit must be a positive safe integer'],
    [{ limit: 1.5 }, 'limit must be a positive safe integer'],
    [{ skip: -1 }, 'skip must be a non-negative safe integer'],
  ])('refuses paging %o', async (request, message) => {
    const run = vi.fn<GitRunner>()
    await expect(new GitCommands(run).log('/repo', request, signal())).rejects.toThrow(message)
    expect(run).not.toHaveBeenCalled()
  })
})

describe('GitCommands show', () => {
  const OID = 'a'.repeat(40)

  /** A runner recording argv, resolving the commit, and answering the patch. */
  function showRunner(patch: string): { run: GitRunner; calls: string[][] } {
    const calls: string[][] = []
    const run: GitRunner = async (args) => {
      calls.push([...args])
      return args[0] === 'rev-parse'
        ? { stdout: `${OID}\n`, stderr: '', exitCode: 0 }
        : { stdout: patch, stderr: '', exitCode: 0 }
    }
    return { run, calls }
  }

  it('resolves the commit and reads its patch, optionally for one path', async () => {
    const { run, calls } = showRunner('diff --git a/a.ts b/a.ts\n')
    const commands = new GitCommands(run)
    await expect(commands.show('/repo', { commit: 'abc1234', path: 'a.ts' }, signal()))
      .resolves.toEqual({ commit: OID, patch: 'diff --git a/a.ts b/a.ts\n', truncated: false })
    expect(calls[0]).toEqual(['rev-parse', '--verify', 'abc1234^{commit}'])
    expect(calls[1]).toEqual(['-c', 'core.quotePath=false', 'show', '--format=', '--no-color', '--no-ext-diff', OID, '--', 'a.ts'])
    await commands.show('/repo', { commit: OID }, signal())
    expect(calls[3]).toEqual(['-c', 'core.quotePath=false', 'show', '--format=', '--no-color', '--no-ext-diff', OID])
  })

  it('flags a patch past the configured byte cap', async () => {
    const { run } = showRunner('x'.repeat(64))
    const shown = await new GitCommands(run, 8).show('/repo', { commit: OID }, signal())
    expect(shown.truncated).toBe(true)
    expect(Buffer.byteLength(shown.patch, 'utf8')).toBeLessThanOrEqual(8)
  })

  it('refuses a commit that is not an object id and a path outside the repository', async () => {
    const run = vi.fn<GitRunner>()
    const commands = new GitCommands(run)
    for (const commit of ['', 'zzzz', 'a'.repeat(65)]) {
      await expect(commands.show('/repo', { commit }, signal())).rejects.toMatchObject({ code: 'gateway/bad-request' })
    }
    await expect(commands.show('/repo', { commit: OID, path: '../x' }, signal()))
      .rejects.toMatchObject({ code: 'git/path-refused' })
    expect(run).not.toHaveBeenCalled()
  })
})

describe('GitCommands remote verbs', () => {
  let sandbox: string
  let remote: string
  let root: string
  let second: string

  beforeEach(async () => {
    sandbox = await scratch('dsh-git-remote-')
    remote = join(sandbox, 'remote.git')
    root = join(sandbox, 'work')
    second = join(sandbox, 'second')
    git(sandbox, 'init', '--bare', '-b', 'main', remote)
    git(sandbox, 'init', '-b', 'main', root)
    await writeFile(join(root, 'a.txt'), 'base\n')
    git(root, 'add', 'a.txt')
    git(root, 'commit', '-m', 'base')
    git(root, 'remote', 'add', 'origin', remote)
    git(root, 'push', '-u', 'origin', 'main')
    git(sandbox, 'clone', remote, second)
  })

  afterEach(async () => {
    await rm(sandbox, { recursive: true, force: true })
  })

  it('fetches, pulls, and pushes through the configured remote', async () => {
    const commands = new GitCommands(RUNNER)
    await expect(commands.fetch(root, signal())).resolves.toMatchObject({ operation: 'fetch', truncated: false })

    git(second, 'checkout', 'main')
    await writeFile(join(second, 'b.txt'), 'remote\n')
    git(second, 'add', 'b.txt')
    git(second, 'commit', '-m', 'remote change')
    git(second, 'push', 'origin', 'main')

    await expect(commands.pull(root, signal())).resolves.toMatchObject({ operation: 'pull' })
    await expect(readFile(join(root, 'b.txt'), 'utf8')).resolves.toBe('remote\n')

    await writeFile(join(root, 'c.txt'), 'local\n')
    git(root, 'add', 'c.txt')
    git(root, 'commit', '-m', 'local change')
    await expect(commands.push(root, {}, signal())).resolves.toMatchObject({ operation: 'push' })
    expect(git(remote, 'rev-parse', 'main').trim()).toBe(git(root, 'rev-parse', 'HEAD').trim())
  })

  it('reports diverged history on pull and push without force', async () => {
    git(second, 'checkout', 'main')
    await writeFile(join(second, 'b.txt'), 'remote\n')
    git(second, 'add', 'b.txt')
    git(second, 'commit', '-m', 'remote change')
    git(second, 'push', 'origin', 'main')

    await writeFile(join(root, 'c.txt'), 'local\n')
    git(root, 'add', 'c.txt')
    git(root, 'commit', '-m', 'local change')

    const commands = new GitCommands(RUNNER)
    await expect(commands.pull(root, signal())).rejects.toMatchObject({ code: 'git/remote-diverged' })
    await expect(commands.push(root, {}, signal())).rejects.toMatchObject({ code: 'git/remote-diverged' })
  })

  it('reports a branch without upstream', async () => {
    git(root, 'checkout', '-b', 'feature')
    await expect(new GitCommands(RUNNER).push(root, {}, signal()))
      .rejects.toMatchObject({ code: 'git/remote-no-upstream' })
  })

  it('reports an unreachable remote as a remote failure', async () => {
    git(root, 'remote', 'set-url', 'origin', join(sandbox, 'missing.git'))
    await expect(new GitCommands(RUNNER).fetch(root, signal()))
      .rejects.toMatchObject({ code: 'git/remote-failed' })
  })

  it.each([
    ['fatal: Authentication failed for repository', 'git/remote-auth'],
    ['fatal: could not read Username for host: terminal prompts disabled', 'git/remote-auth'],
    ['! [rejected] main -> main (non-fast-forward)', 'git/remote-diverged'],
    ['fatal: Not possible to fast-forward, aborting.', 'git/remote-diverged'],
    ['fatal: The current branch main has no upstream branch', 'git/remote-no-upstream'],
    ['fatal: something else', 'git/remote-failed'],
  ])('classifies %s as %s', async (stderr, code) => {
    const network: GitRunner = async () => ({ stdout: '', stderr, exitCode: 1 })
    const commands = new GitCommands(RUNNER, 1024, network, 1024)
    await expect(commands.fetch('/repo', signal())).rejects.toMatchObject({ code })
  })

  it('names a remote failure that carries no stderr', async () => {
    const network: GitRunner = async () => ({ stdout: '', stderr: '', exitCode: 1 })
    const commands = new GitCommands(RUNNER, 1024, network, 1024)
    await expect(commands.fetch('/repo', signal())).rejects.toThrow('git exited nonzero')
  })

  it('prefers git stderr and falls back to stdout for the summary', async () => {
    const stderrRun = new GitCommands(RUNNER, 1024, async () => ({ stdout: '', stderr: 'progress', exitCode: 0 }), 1024)
    await expect(stderrRun.fetch('/repo', signal())).resolves.toEqual({ operation: 'fetch', summary: 'progress', truncated: false })
    const stdoutRun = new GitCommands(RUNNER, 1024, async () => ({ stdout: 'stdout text', stderr: '', exitCode: 0 }), 1024)
    await expect(stdoutRun.fetch('/repo', signal())).resolves.toEqual({ operation: 'fetch', summary: 'stdout text', truncated: false })
  })

  it('flags a summary past the configured byte cap', async () => {
    const commands = new GitCommands(RUNNER, 1024, async () => ({ stdout: '', stderr: 'x'.repeat(64), exitCode: 0 }), 8)
    const value = await commands.fetch('/repo', signal())
    expect(value.truncated).toBe(true)
    expect(Buffer.byteLength(value.summary, 'utf8')).toBeLessThanOrEqual(8)
  })

  it('defaults the network runner and summary cap', async () => {
    const commands = new GitCommands(RUNNER)
    await expect(commands.fetch(root, signal())).resolves.toMatchObject({ operation: 'fetch', truncated: false })
  })
})

describe('GitCommands operation state and steps', () => {
  let root: string

  beforeEach(async () => {
    root = await scratch('dsh-git-operation-')
    git(root, 'init', '-b', 'main')
    await writeFile(join(root, 'a.txt'), 'base\n')
    git(root, 'add', 'a.txt')
    git(root, 'commit', '-m', 'base')
  })

  afterEach(async () => {
    await rm(root, { recursive: true, force: true })
  })

  /** Leave `feature` and `main` modifying the same line so rebase or merge conflicts. */
  async function conflictingBranch(): Promise<void> {
    git(root, 'checkout', '-b', 'feature')
    await writeFile(join(root, 'a.txt'), 'feature\n')
    git(root, 'commit', '-am', 'feature')
    git(root, 'checkout', 'main')
    await writeFile(join(root, 'a.txt'), 'main\n')
    git(root, 'commit', '-am', 'main')
    git(root, 'checkout', 'feature')
  }

  it('reports no operation in a clean repository', async () => {
    await expect(new GitCommands(RUNNER).state(root, signal()))
      .resolves.toEqual({ operation: 'none', conflicts: [] })
  })

  it('resolves an absolute git directory from a linked worktree', async () => {
    const worktree = join(root, '..', `wt-${Date.now()}`)
    git(root, 'worktree', 'add', '-b', 'linked', worktree)
    try {
      await expect(new GitCommands(RUNNER).state(worktree, signal()))
        .resolves.toEqual({ operation: 'none', conflicts: [] })
    } finally {
      git(root, 'worktree', 'remove', '--force', worktree)
    }
  })

  it('refuses a step that fails without leaving conflicts', async () => {
    const run: GitRunner = async (args) => {
      if (args[0] === 'rebase') return { stdout: '', stderr: '', exitCode: 1 }
      if (args[0] === 'for-each-ref') return { stdout: 'main\tabc\t*\n', stderr: '', exitCode: 0 }
      if (args[0] === 'status') return { stdout: '# branch.head main\0', stderr: '', exitCode: 0 }
      if (args[0] === 'rev-parse') return { stdout: '.git', stderr: '', exitCode: 0 }
      return { stdout: '', stderr: '', exitCode: 0 }
    }
    await expect(new GitCommands(run).rebase('/repo', { branch: 'main' }, signal()))
      .rejects.toThrow('exit 1')
  })

  it('reports a rebase conflict and keeps unresolved conflicts until resolved', async () => {
    await conflictingBranch()
    const commands = new GitCommands(RUNNER)
    const started = await commands.rebase(root, { branch: 'main' }, signal())
    expect(started.state).toEqual({ operation: 'rebase', conflicts: ['a.txt'] })
    await expect(commands.state(root, signal())).resolves.toEqual({ operation: 'rebase', conflicts: ['a.txt'] })

    const stillConflicted = await commands.continueOperation(root, signal())
    expect(stillConflicted.state.conflicts).toEqual(['a.txt'])

    await writeFile(join(root, 'a.txt'), 'resolved\n')
    await commands.stage(root, { path: 'a.txt' }, signal())
    const finished = await commands.continueOperation(root, signal())
    expect(finished.state).toEqual({ operation: 'none', conflicts: [] })
    await expect(commands.status(root, signal())).resolves.toMatchObject({ branch: 'feature', files: [] })
  })

  it('aborts a rebase and restores the branch tip', async () => {
    await conflictingBranch()
    const commands = new GitCommands(RUNNER)
    const before = git(root, 'rev-parse', 'HEAD').trim()
    await commands.rebase(root, { branch: 'main' }, signal())
    await expect(commands.abortOperation(root, signal()))
      .resolves.toMatchObject({ state: { operation: 'none', conflicts: [] } })
    expect(git(root, 'rev-parse', 'HEAD').trim()).toBe(before)
  })

  it('refuses a rebase onto an invalid or unknown branch and a dirty tree', async () => {
    const commands = new GitCommands(RUNNER)
    for (const branch of ['-x', 'two words', '', 'missing']) {
      await expect(commands.rebase(root, { branch }, signal()))
        .rejects.toMatchObject({ code: 'git/operation-refused' })
    }
    await conflictingBranch()
    git(root, 'checkout', 'main')
    await writeFile(join(root, 'a.txt'), 'dirty\n')
    await expect(commands.rebase(root, { branch: 'feature' }, signal()))
      .rejects.toMatchObject({ code: 'git/operation-refused' })
  })

  it('merges a branch and reports the resulting state', async () => {
    git(root, 'checkout', '-b', 'topic')
    await writeFile(join(root, 'b.txt'), 'topic\n')
    git(root, 'add', 'b.txt')
    git(root, 'commit', '-m', 'topic change')
    git(root, 'checkout', 'main')
    const merged = await new GitCommands(RUNNER).merge(root, { branch: 'topic' }, signal())
    expect(merged.state).toEqual({ operation: 'none', conflicts: [] })
    await expect(readFile(join(root, 'b.txt'), 'utf8')).resolves.toBe('topic\n')
  })

  it('keeps a conflicted merge until the resolution is staged', async () => {
    await conflictingBranch()
    const commands = new GitCommands(RUNNER)
    const started = await commands.merge(root, { branch: 'main' }, signal())
    expect(started.state).toEqual({ operation: 'merge', conflicts: ['a.txt'] })
    await writeFile(join(root, 'a.txt'), 'resolved\n')
    await commands.stage(root, { path: 'a.txt' }, signal())
    await expect(commands.continueOperation(root, signal()))
      .resolves.toMatchObject({ state: { operation: 'none', conflicts: [] } })
  })

  it('aborts a conflicted merge and restores the branch tip', async () => {
    await conflictingBranch()
    const commands = new GitCommands(RUNNER)
    const before = git(root, 'rev-parse', 'HEAD').trim()
    await commands.merge(root, { branch: 'main' }, signal())
    await expect(commands.abortOperation(root, signal()))
      .resolves.toMatchObject({ state: { operation: 'none', conflicts: [] } })
    expect(git(root, 'rev-parse', 'HEAD').trim()).toBe(before)
  })

  it('reverts one commit and reports its success', async () => {
    await writeFile(join(root, 'b.txt'), 'added\n')
    git(root, 'add', 'b.txt')
    git(root, 'commit', '-m', 'add b')
    const commit = git(root, 'rev-parse', 'HEAD').trim()
    const reverted = await new GitCommands(RUNNER).revert(root, { commit }, signal())
    expect(reverted.state).toEqual({ operation: 'none', conflicts: [] })
    await expect(readFile(join(root, 'b.txt'), 'utf8')).rejects.toThrow()
  })

  it('continues a conflicted revert after the resolution is staged', async () => {
    await writeFile(join(root, 'a.txt'), 'second\n')
    git(root, 'commit', '-am', 'second')
    const second = git(root, 'rev-parse', 'HEAD').trim()
    await writeFile(join(root, 'a.txt'), 'third\n')
    git(root, 'commit', '-am', 'third')
    const commands = new GitCommands(RUNNER)
    const started = await commands.revert(root, { commit: second }, signal())
    expect(started.state).toEqual({ operation: 'revert', conflicts: ['a.txt'] })
    await writeFile(join(root, 'a.txt'), 'resolved\n')
    await commands.stage(root, { path: 'a.txt' }, signal())
    await expect(commands.continueOperation(root, signal()))
      .resolves.toMatchObject({ state: { operation: 'none', conflicts: [] } })
  })

  it('aborts a conflicted revert and restores the branch tip', async () => {
    await writeFile(join(root, 'a.txt'), 'second\n')
    git(root, 'commit', '-am', 'second')
    const second = git(root, 'rev-parse', 'HEAD').trim()
    await writeFile(join(root, 'a.txt'), 'third\n')
    git(root, 'commit', '-am', 'third')
    const commands = new GitCommands(RUNNER)
    const before = git(root, 'rev-parse', 'HEAD').trim()
    await commands.revert(root, { commit: second }, signal())
    await expect(commands.abortOperation(root, signal()))
      .resolves.toMatchObject({ state: { operation: 'none', conflicts: [] } })
    expect(git(root, 'rev-parse', 'HEAD').trim()).toBe(before)
  })

  it('cherry-picks one commit and reports its success', async () => {
    git(root, 'checkout', '-b', 'side')
    await writeFile(join(root, 'b.txt'), 'side\n')
    git(root, 'add', 'b.txt')
    git(root, 'commit', '-m', 'side change')
    const commit = git(root, 'rev-parse', 'HEAD').trim()
    git(root, 'checkout', 'main')

    const picked = await new GitCommands(RUNNER).cherryPick(root, { commit }, signal())
    expect(picked.state).toEqual({ operation: 'none', conflicts: [] })
    await expect(readFile(join(root, 'b.txt'), 'utf8')).resolves.toBe('side\n')
  })

  it('continues a cherry-pick after the conflict is staged', async () => {
    await conflictingBranch()
    const side = git(root, 'rev-parse', 'HEAD').trim()
    git(root, 'checkout', 'main')
    const commands = new GitCommands(RUNNER)
    await commands.cherryPick(root, { commit: side }, signal())
    await writeFile(join(root, 'a.txt'), 'resolved\n')
    await commands.stage(root, { path: 'a.txt' }, signal())
    await expect(commands.continueOperation(root, signal()))
      .resolves.toMatchObject({ state: { operation: 'none', conflicts: [] } })
  })

  it('reports a cherry-pick conflict and aborts it', async () => {
    await conflictingBranch()
    const side = git(root, 'rev-parse', 'HEAD').trim()
    git(root, 'checkout', 'main')
    const commands = new GitCommands(RUNNER)
    const started = await commands.cherryPick(root, { commit: side }, signal())
    expect(started.state.operation).toBe('cherry-pick')
    expect(started.state.conflicts).toEqual(['a.txt'])
    await expect(commands.abortOperation(root, signal()))
      .resolves.toMatchObject({ state: { operation: 'none', conflicts: [] } })
  })

  it('refuses an invalid commit id and a step with no operation in progress', async () => {
    const commands = new GitCommands(RUNNER)
    await expect(commands.cherryPick(root, { commit: 'not-a-commit' }, signal()))
      .rejects.toMatchObject({ code: 'git/operation-refused' })
    for (const branch of ['-x', 'two words', '', 'missing']) {
      await expect(commands.merge(root, { branch }, signal()))
        .rejects.toMatchObject({ code: 'git/operation-refused' })
    }
    await expect(commands.revert(root, { commit: 'not-a-commit' }, signal()))
      .rejects.toMatchObject({ code: 'git/operation-refused' })
    await expect(commands.continueOperation(root, signal()))
      .rejects.toMatchObject({ code: 'git/operation-refused' })
    await expect(commands.abortOperation(root, signal()))
      .rejects.toMatchObject({ code: 'git/operation-refused' })
  })

  it('reports an in-progress merge', async () => {
    await conflictingBranch()
    gitTolerating(root, 'merge', 'main')
    try {
      await expect(new GitCommands(RUNNER).state(root, signal()))
        .resolves.toMatchObject({ operation: 'merge', conflicts: ['a.txt'] })
    } finally {
      git(root, 'merge', '--abort')
    }
  })

  it('reports an in-progress revert', async () => {
    await writeFile(join(root, 'a.txt'), 'second\n')
    git(root, 'commit', '-am', 'second')
    const second = git(root, 'rev-parse', 'HEAD').trim()
    await writeFile(join(root, 'a.txt'), 'third\n')
    git(root, 'commit', '-am', 'third')
    gitTolerating(root, 'revert', '--no-edit', second)
    try {
      await expect(new GitCommands(RUNNER).state(root, signal()))
        .resolves.toMatchObject({ operation: 'revert', conflicts: ['a.txt'] })
    } finally {
      git(root, 'revert', '--abort')
    }
  })
})
