/**
 * Shared no-shell `execFile` runner for host-native OS integrations.
 * @module @deepseek-ai/dsh-native-command/runner
 */

import { execFile } from 'node:child_process'

/** Testable command boundary; native implementations never invoke a shell. */
export type NativeCommandRunner = (
  command: string,
  args: readonly string[],
  signal: AbortSignal,
  window: 'hidden' | 'visible',
  env?: Readonly<Record<string, string>>,
) => Promise<{ stdout: string; stderr: string }>

/**
 * Run a host command with utf8 stdio, abort propagation, and explicit GUI visibility.
 * @param command - executable path or PATH name.
 * @param args - argv (never a shell string).
 * @param signal - caller/connection lifetime; abort terminates the child.
 * @param window - Windows startup visibility: `hidden` for background commands,
 * `visible` for GUI launchers. Ignored on other platforms.
 * @param env - environment entries laid over the Host's own environment; omitted keeps it unchanged.
 * @returns captured stdout/stderr on exit 0.
 */
export const runNativeCommand: NativeCommandRunner = (command, args, signal, window, env) =>
  new Promise((resolve, reject) => {
    execFile(
      command,
      [...args],
      {
        encoding: 'utf8',
        signal,
        windowsHide: window === 'hidden',
        ...(env === undefined ? {} : { env: { ...process.env, ...env } }),
      },
      (error, stdout, stderr) => {
        if (error !== null) {
          const failure = Object.assign(new Error(error.message, { cause: error }), {
            code: error.code,
            stdout,
            stderr,
          })
          reject(failure)
          return
        }
        resolve({ stdout, stderr })
      },
    )
  })
