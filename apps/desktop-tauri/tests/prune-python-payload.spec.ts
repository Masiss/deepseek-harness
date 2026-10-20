import { lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { prunePythonPayload } from '../scripts/prune-python-payload.ts'

let root: string

function touch(relativePath: string): void {
  const path = join(root, relativePath)
  mkdirSync(join(path, '..'), { recursive: true })
  writeFileSync(path, '')
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), 'dsh-python-prune-'))
  for (const path of [
    'include/python3.12/Python.h',
    'share/man/man1/python.1',
    'share/terminfo/x/xterm',
    'lib/python3.12/json/__init__.py',
    'lib/python3.12/ensurepip/__init__.py',
    'lib/python3.12/idlelib/__init__.py',
    'lib/python3.12/lib2to3/__init__.py',
    'lib/python3.12/pydoc_data/topics.py',
    'lib/python3.12/tkinter/__init__.py',
    'lib/python3.12/site-packages/pip/__init__.py',
    'lib/python3.12/site-packages/numpy/__init__.py',
    'lib/python3.12/lib-dynload/_tkinter.cpython-312-x86_64-linux-gnu.so',
    'lib/python3.12/lib-dynload/_ssl.cpython-312-x86_64-linux-gnu.so',
    'lib/tcl9.0/init.tcl',
    'lib/tk9.0/tk.tcl',
    'lib/itcl4.3.8/itcl.tcl',
    'lib/thread3.0.6/pkgIndex.tcl',
    'lib/libtcl9.0.so',
    'lib/libtcl9tk9.0.so',
  ]) touch(path)
  // Two real copies of the shared library, as the locked payload ships them.
  touch('lib/libpython3.12.so')
  touch('lib/libpython3.12.so.1.0')
})

afterEach(() => {
  rmSync(root, { recursive: true, force: true })
})

describe('prunePythonPayload', () => {
  it('removes packaging and development surface and keeps documented modules', () => {
    const result = prunePythonPayload(root)

    expect(result.deduplicatedLibrary).toBe(true)
    expect(result.removed).toEqual(expect.arrayContaining([
      'include', 'share/man', 'share/terminfo',
      'lib/python3.12/ensurepip', 'lib/python3.12/idlelib', 'lib/python3.12/lib2to3',
      'lib/python3.12/pydoc_data', 'lib/python3.12/tkinter', 'lib/python3.12/site-packages/pip',
      'lib/python3.12/lib-dynload/_tkinter.cpython-312-x86_64-linux-gnu.so',
      'lib/tcl9.0', 'lib/tk9.0', 'lib/itcl4.3.8', 'lib/thread3.0.6', 'lib/libtcl9.0.so', 'lib/libtcl9tk9.0.so',
    ]))

    expect(lstatSync(join(root, 'lib/libpython3.12.so')).isSymbolicLink()).toBe(true)
    expect(readFileSync(join(root, 'lib/libpython3.12.so.1.0')).length).toBe(0)
    expect(lstatSync(join(root, 'lib/python3.12/site-packages/numpy')).isDirectory()).toBe(true)
    expect(readFileSync(join(root, 'lib/python3.12/json/__init__.py')).length).toBe(0)
    expect(lstatSync(join(root, 'lib/python3.12/lib-dynload/_ssl.cpython-312-x86_64-linux-gnu.so')).isFile()).toBe(true)
  })

  it('is idempotent', () => {
    prunePythonPayload(root)
    expect(prunePythonPayload(root)).toEqual({ removed: [], deduplicatedLibrary: false })
  })

  it('rejects a tree that does not hold exactly one interpreter version', () => {
    touch('lib/python3.11/os.py')
    expect(() => prunePythonPayload(root)).toThrow(/expected one lib\/python3.N directory/u)
  })
})
