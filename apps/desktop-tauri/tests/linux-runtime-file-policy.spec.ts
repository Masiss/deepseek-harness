import { describe, expect, it } from 'vitest'
import { linuxRuntimeFileExclusion } from '../scripts/linux-runtime-file-policy.ts'

const linux = { platform: 'linux' as const, rustProvider: true }

describe('linuxRuntimeFileExclusion', () => {
  it('drops packages the Desktop profile never mounts', () => {
    expect(linuxRuntimeFileExclusion(
      '@deepseek-ai/dsh-experimental-inspector/lib/index.js',
      linux,
    )).toBe('experimental package the Desktop profile does not mount')
    expect(linuxRuntimeFileExclusion(
      '@deepseek-ai/dsh-experimental-speech-to-text-sensevoice/package.json',
      linux,
    )).toContain('experimental package')
  })

  it('drops the local speech model only that experimental bundle uses', () => {
    expect(linuxRuntimeFileExclusion('sherpa-onnx-linux-x64/package.json', linux))
      .toContain('speech model')
    expect(linuxRuntimeFileExclusion('sherpa-onnx-node/lib/index.js', linux))
      .toContain('speech model')
  })

  it('keeps a package the mounted profile resolves', () => {
    expect(linuxRuntimeFileExclusion('@deepseek-ai/dsh-web-app/lib/index.js', linux)).toBeUndefined()
    expect(linuxRuntimeFileExclusion('@deepseek-ai/dsh-experimentalx/lib/index.js', linux)).toBeUndefined()
    expect(linuxRuntimeFileExclusion('sherpa-onnx', linux)).toBeUndefined()
  })

  it('leaves another platform alone', () => {
    expect(linuxRuntimeFileExclusion(
      '@deepseek-ai/dsh-experimental-inspector/lib/index.js',
      { platform: 'darwin', rustProvider: true },
    )).toBeUndefined()
  })
})
