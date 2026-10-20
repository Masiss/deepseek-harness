'use strict'
/**
 * `koffi`-shaped entry over the Rust native module.
 *
 * The Linux one-shot process bootstrap binds libc's `execve` and `fcntl`
 * through koffi. This shim serves exactly those two signatures from the Rust
 * module, so the Linux payload carries no FFI library. A signature the shim
 * does not know is refused rather than silently ignored.
 */

const native = require('./dsh-native.node')

/** The libc signatures the Harness binds, mapped to their Rust counterpart. */
const SIGNATURES = new Map([
  [
    'int execve(const char *pathname, const char **argv, const char **envp)',
    (file, argv, envp) => native.execve(file, withoutTerminator(argv), withoutTerminator(envp)),
  ],
  [
    'int fcntl(int fd, int cmd, int arg)',
    (fd, cmd, arg) => native.fcntl(fd, cmd, arg),
  ],
])

/**
 * Drop the null terminator the caller appends to a C string array.
 * @param values - Array the caller built for a C string array.
 * @returns The same values without their trailing null.
 */
function withoutTerminator(values) {
  return values.filter(value => value !== null && value !== undefined)
}

/**
 * Load a library's symbols.
 * @param library - `null` selects the process's libc.
 * @returns The symbol table of that library.
 */
function load(library) {
  if (library !== null) {
    throw new Error(`koffi shim: only the process libc is available, got ${String(library)}`)
  }
  return {
    func(signature) {
      const binding = SIGNATURES.get(signature)
      if (binding === undefined) {
        throw new Error(`koffi shim: unsupported signature ${signature}`)
      }
      return binding
    },
  }
}

/** Errno of the last failed syscall. */
function errno() {
  return native.lastErrno()
}

module.exports = { load, errno, default: { load, errno } }
