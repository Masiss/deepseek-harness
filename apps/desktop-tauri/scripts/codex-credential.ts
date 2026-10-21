/**
 * Copy a Codex OAuth credential from the local pi credential store into the
 * Harness credential store.
 *
 * The `openai-codex` provider is OAuth-only: it reads a ChatGPT access token
 * and takes the account id from the token's claims. The Harness credential
 * store holds that token as a grant record the provider's own adapter reads,
 * so putting the credential there is all it takes to route a request through
 * the provider.
 *
 * The source credential is the one the local pi agent already holds, so no
 * browser login is needed.
 *
 *   usage: codex-credential.ts [--check]
 */
import { readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import { credentialKey } from '@deepseek-ai/dsh-credentials'
import { LocalCredentialProvider } from '@deepseek-ai/dsh-credentials-local'

interface StoredOAuth {
  readonly type: 'oauth'
  readonly access: string
  readonly refresh: string
  readonly expires: number
  readonly accountId: string
}

const piStore = join(homedir(), '.pi', 'agent', 'auth.json')

/** The Codex credential the local pi agent holds, or undefined when there is none. */
function readPiCredential(): StoredOAuth | undefined {
  const document = JSON.parse(readFileSync(piStore, 'utf8')) as Record<string, unknown>
  const entry = document['openai-codex'] as Partial<StoredOAuth> | undefined
  if (entry === undefined) return undefined
  const { type, access, refresh, expires, accountId } = entry
  if (type !== 'oauth' || typeof access !== 'string' || typeof refresh !== 'string'
    || typeof expires !== 'number' || typeof accountId !== 'string') return undefined
  return { type: 'oauth', access, refresh, expires, accountId }
}

const credential = readPiCredential()
if (credential === undefined) {
  console.error('codex-credential: the local pi store holds no Codex credential')
  process.exit(1)
}
console.log(`codex-credential: source expires in ${Math.round((credential.expires - Date.now()) / 3_600_000)} hours`)

if (process.argv.includes('--check')) {
  console.log('codex-credential: nothing written (--check)')
  process.exit(0)
}

const ctx = new Context()
await ctx.plugin(LocalCredentialProvider, {
  path: join(process.env.DSH_HOME ?? join(homedir(), '.dsh'), '.credentials.yaml'),
  watch: false,
})
const store = ctx.get('credentials')
if (store === undefined) throw new Error('codex-credential: the credentials service did not register')
const key = credentialKey('llm-pi-ai', 'openai-codex')
await store.modifyRecord(key, async () => ({
  kind: 'grant',
  payload: credential,
}))
console.log(`codex-credential: wrote the grant record ${key}`)
