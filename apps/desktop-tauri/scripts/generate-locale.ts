/**
 * Generate the shell's copy from the Electron shell's locale.
 *
 * `apps/desktop/src/locale.ts` owns the English and Chinese copy of the
 * desktop application. The Tauri shell shows the same dialogs, so it reads the
 * same strings instead of keeping a second copy in Rust: this script writes
 * `src-tauri/src/locale.rs`, and the shell looks the keys up there.
 *
 *   usage: bun apps/desktop-tauri/scripts/generate-locale.ts [--check]
 */

import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { en, zh } from '../../desktop/src/locale.ts'

const here = dirname(fileURLToPath(import.meta.url))
const output = resolve(here, '..', 'src-tauri', 'src', 'locale.rs')

/** The keys the Tauri shell shows, in the order it uses them. */
export const SHELL_KEYS = [
  'startupFailed',
  'fatalSummary',
  'quitTitle',
  'quitActiveTasks',
  'quit',
  'cancel',
  'updateTitle',
  'updateAvailable',
  'updateDetail',
  'updateDownloadedTitle',
  'updateDownloadedDetail',
  'updateDownload',
  'updateStopTasks',
  'updateLater',
  'updateAcknowledge',
] as const satisfies readonly (keyof typeof en)[]

/** One locale's copy for the shell's keys. */
function rows(source: Readonly<Record<string, string>>): string {
  return SHELL_KEYS.map((key) => {
    const value = source[key]
    if (typeof value !== 'string') throw new Error(`desktop locale: the shell needs the key ${key}`)
    return `    ("${key}", ${JSON.stringify(value)}),`
  }).join('\n')
}

/** The generated Rust module. */
function module(): string {
  return `//! Copy for the desktop shell, generated from \`apps/desktop/src/locale.ts\`.
//!
//! Regenerate with \`bun apps/desktop-tauri/scripts/generate-locale.ts\`.

/// The English copy of the shell's dialogs.
pub const EN: &[(&str, &str)] = &[
${rows(en)}
];

/// The Chinese copy of the shell's dialogs.
pub const ZH: &[(&str, &str)] = &[
${rows(zh)}
];

/// Look a key up in one locale, falling back to English.
fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> &'static str {
    table
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| *value)
        .unwrap_or("")
}

/// The copy for one key in the session's language.
///
/// The session's language follows the environment; an unknown language reads
/// the English copy.
pub fn text(key: &str) -> &'static str {
    let language = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_default()
        .to_lowercase();
    let chinese = language.starts_with("zh");
    let table = if chinese { ZH } else { EN };
    let value = lookup(table, key);
    if value.is_empty() {
        lookup(EN, key)
    } else {
        value
    }
}
`
}

const generated = module()
if (process.argv.includes('--check')) {
  const current = readFileSync(output, 'utf8')
  if (current !== generated) {
    console.error('desktop locale: the generated copy is stale; run bun apps/desktop-tauri/scripts/generate-locale.ts')
    process.exit(1)
  }
  console.log('desktop locale: the generated copy is up to date')
} else {
  writeFileSync(output, generated)
  console.log(`desktop locale: wrote ${join('src-tauri', 'src', 'locale.rs')}`)
}
