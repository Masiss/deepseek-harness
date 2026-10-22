import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

const root = join(import.meta.dirname, '..')

describe('release shell security configuration', () => {
  it('keeps a restrictive CSP on the bundled pages', () => {
    const config = JSON.parse(readFileSync(join(root, 'src-tauri', 'tauri.conf.json'), 'utf8')) as {
      app?: { security?: { csp?: string } }
    }
    const csp = config.app?.security?.csp
    expect(csp).toBeTypeOf('string')
    expect(csp).toContain("default-src 'self'")
    expect(csp).toContain("object-src 'none'")
    expect(csp).not.toContain('unsafe-eval')
  })

  it('does not compile developer tools into the release shell', () => {
    const cargo = readFileSync(join(root, 'src-tauri', 'Cargo.toml'), 'utf8')
    const tauri = cargo.split('\n').filter(line => line.startsWith('tauri ='))
    expect(tauri).toHaveLength(1)
    expect(tauri[0]).not.toContain('devtools')
  })
})
