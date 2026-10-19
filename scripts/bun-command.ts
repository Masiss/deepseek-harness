/**
 * Resolve Bun as the command prefix for the Web build path, so its steps spawn
 * without a shell and without depending on an interactive `bun` on `PATH`.
 */

/** Bun's lifecycle variable, set by `bun run` to the executing Bun binary. */
const BUN_EXECUTABLE = /[\\/]bun(?:\.exe)?$/iu

/**
 * Resolve Bun as a command prefix that spawns without a shell.
 * @param environment - Environment supplying the `bun run` lifecycle variable.
 * @returns Command and leading arguments to prepend before Bun's arguments.
 */
export function bunCommand(environment: NodeJS.ProcessEnv = process.env): readonly [command: string, ...args: string[]] {
  const entrypoint = environment.npm_execpath
  // Bun's binary is native, so the lifecycle variable names the executable
  // itself and is passed through unchanged rather than run under `process.execPath`.
  if (entrypoint !== undefined && entrypoint !== '' && BUN_EXECUTABLE.test(entrypoint)) return [entrypoint]
  return ['bun']
}

/**
 * Resolve Bun together with the arguments for one Web build invocation.
 * @param args - Arguments passed to Bun, without shell quoting.
 * @param environment - Environment supplying the `bun run` lifecycle variable.
 * @returns A command and argument array suitable for `execa` without a shell.
 */
export function bunInvocation(
  args: readonly string[],
  environment: NodeJS.ProcessEnv = process.env,
): { command: string; args: string[] } {
  const [command, ...prefix] = bunCommand(environment)
  return { command, args: [...prefix, ...args] }
}
