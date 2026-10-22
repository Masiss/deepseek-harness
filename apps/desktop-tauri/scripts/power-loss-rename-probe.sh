#!/usr/bin/env bash
# Kill a component commit after the version-directory rename and recover it.
set -u
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/dsh-power-loss-rename.XXXXXX")
trap 'rm -rf "$work"' EXIT
helper="$work/probe.mjs"
cat > "$helper" <<'JS'
import { mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
const { DesktopOfficeComponent } = await import(process.argv[2])
const [mode, directory] = process.argv.slice(3)
const versions = join(directory, 'versions')
const oldTree = join(versions, 'old')
const staging = join(directory, 'staging-crash')
const target = join(versions, 'new')
const backup = join(versions, '.backup-crash')
const journal = join(directory, 'rename-journal.json')
if (mode === 'crash') {
  mkdirSync(join(oldTree), { recursive: true })
  mkdirSync(staging, { recursive: true })
  writeFileSync(join(oldTree, 'tree'), 'old')
  writeFileSync(join(staging, 'tree'), 'new')
  writeFileSync(join(directory, 'current.json'), '{"version":"old"}\n')
  writeFileSync(journal, JSON.stringify({ oldVersion: 'old', version: 'new', target, backup,
    markerTemporary: join(directory, 'current.json.tmp'), staging }))
  renameSync(staging, target)
  process.kill(process.pid, 'SIGKILL')
}
if (mode === 'recover') {
  const component = new DesktopOfficeComponent({ feed: 'http://127.0.0.1', directory, primaryRuntime: directory })
  await component.recover()
  const current = JSON.parse(readFileSync(join(directory, 'current.json'), 'utf8')).version
  const entries = readdirSync(versions)
  if (current !== 'old' || entries.length !== 1 || entries[0] !== 'old' || readdirSync(directory).includes('rename-journal.json')) process.exit(1)
}
JS
mkdir -p "$work/components"
set +e
bun "$helper" "$repo/apps/desktop-host/src/components/office.ts" crash "$work/components"
crash_status=$?
set -e
set +e
bun "$helper" "$repo/apps/desktop-host/src/components/office.ts" recover "$work/components"
recover_status=$?
set -e
if [ "$crash_status" -eq 137 ] && [ "$recover_status" -eq 0 ]; then
  echo 'RESULT power-loss-rename=OK old-or-new=old journal=clean'
  exit 0
fi
echo "RESULT power-loss-rename=FAIL crash_status=$crash_status recover_status=$recover_status"
exit 1
