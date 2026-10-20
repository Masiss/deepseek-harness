/**
 * Load probe: a plugin whose only job is to prove it ran.
 *
 * It is shipped as a feature bundle (`dsh.bundle.patch`) so a profile can
 * install it next to the Web application bundle instead of replacing it, which
 * is what every published application bundle does.
 */

export const name = '@masiss/dsh-load-probe'

/** Announce the row on the Host's stderr, where probes read it. */
export function apply() {
  console.error('DSH_LOAD_PROBE_ACTIVE')
}
