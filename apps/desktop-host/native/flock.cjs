'use strict'
/**
 * `flock` entry over the Rust native module.
 *
 * The session lease imports `@deepseek-ai/node-addon-system/flock` and takes an
 * exclusive, nonblocking POSIX lock on a descriptor it owns. This wrapper
 * keeps that entry's contract: the syscall never opens, duplicates, or closes
 * the descriptor, and a failure carries the code, the positive errno, and the
 * syscall name.
 */

const { getSystemErrorName } = require('node:util')
const native = require('./dsh-native.node')

async function tryLockExclusive(fd) {
  const errno = native.tryLockExclusive(fd)
  if (errno === 0) return
  const code = getSystemErrorName(-errno)
  throw Object.assign(new Error(`${code}: flock failed`), {
    code,
    errno,
    syscall: 'flock',
  })
}

module.exports = { tryLockExclusive }
