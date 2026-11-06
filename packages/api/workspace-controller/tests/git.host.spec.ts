/** Git Remote contract: porcelain parsing, repository status, and checkout. */
import { execFileSync } from 'node:child_process'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SessionStore, { SESSION_FORMAT_VERSION, SessionId } from '@deepseek-ai/dsh-session'
import TypertRegistry from '@deepseek-ai/dsh-typert-registry'
import { RemoteError } from '@deepseek-ai/dsh-typert-protocol'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { GitCommands, GitController, nativeGitRunner, parseGitBranches, parseGitStatus, type GitRunner } from '../src/git.ts'

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
      files: [
        { path: 'src/app.ts', status: 'modified', staged: false, worktree: true },
        { path: 'notes.md', status: 'untracked', staged: false, worktree: true },
      ],
    })
  })

  it('reports detached HEAD without inventing a branch', () => {
    const status = parseGitStatus('# branch.oid abc\0# branch.head (detached)\0')
    expect(status).toMatchObject({ branch: null, commit: 'abc', detached: true })
  })

  it('reports an unborn branch with no commit', () => {
    const status = parseGitStatus('# branch.oid (initial)\0# branch.head main\0')
    expect(status).toMatchObject({ branch: 'main', commit: null, detached: false })
  })

  it('keeps a rename path and skips its original-path record', () => {
    const output = [
      '# branch.head main',
      '2 R. N... 100644 100644 100644 aaa bbb R100 new name.ts',
      'old name.ts',
      '',
    ].join('\0')
    expect(parseGitStatus(output).files).toEqual([{ path: 'new name.ts', status: 'renamed', staged: true, worktree: false }])
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
      { path: 'staged.ts', status: 'added', staged: true, worktree: false },
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
    const plugin = await ctx.plugin(GitController, { timeoutMs: 5_000 })
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

      await expect(controller.state(scope, signal())).resolves.toEqual({ operation: 'none', conflicts: [] })
      await expect(controller.rebase(scope, { branch: 'main' }, signal()))
        .resolves.toMatchObject({ state: { operation: 'none' } })
      git(root, 'checkout', 'main')
      await expect(controller.cherryPick(scope, { commit: committed.commit }, signal()))
        .resolves.toMatchObject({ state: { operation: 'none' } })
      await expect(controller.fetch(scope, signal())).resolves.toMatchObject({ operation: 'fetch' })
      await expect(controller.pull(scope, signal())).resolves.toMatchObject({ operation: 'pull' })
      await expect(controller.push(scope, signal())).resolves.toMatchObject({ operation: 'push' })
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
    await expect(commands.push(root, signal())).resolves.toMatchObject({ operation: 'push' })
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
    await expect(commands.push(root, signal())).rejects.toMatchObject({ code: 'git/remote-diverged' })
  })

  it('reports a branch without upstream', async () => {
    git(root, 'checkout', '-b', 'feature')
    await expect(new GitCommands(RUNNER).push(root, signal()))
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
