#!/usr/bin/env bash
# Verify how the packaged runtime resolves the Office engine.
#
# The engine travels with the on-demand Office component, so the loader points
# the engine's package at the installed copy. A runtime that carries neither
# the component nor the engine points the package at a placeholder that
# reports the missing component, so the plugins that need the engine load
# instead of failing to import. The probe resolves the engine through the
# loader in both states.
#
#   usage: office-engine-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
loader="$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/native/native-loader.mjs"
[ -f "$loader" ] || { echo "RESULT engine=missing"; exit 1; }

script="$tree/dsh/probe/office-engine-probe.mjs"
work="$tree/probe/office-engine"
mkdir -p "$(dirname "$script")" "$work"

cat > "$script" <<'PROBE'
import { createConverter } from '@deepseek-ai/libreoffice-kit'

const resolved = import.meta.resolve('@deepseek-ai/libreoffice-kit')
console.log('RESULT engine=' + (resolved.includes('office-engine-stub') ? 'placeholder' : resolved))
try {
  await createConverter()
  console.log('RESULT conversion=accepted')
} catch (error) {
  console.log('RESULT conversion=' + (error?.code ?? 'failed'))
}
PROBE

# A runtime without the component reports the missing engine.
missing="$work/without-component"
rm -rf "$missing"
mkdir -p "$missing"
DSH_HOME="$missing" "$tree/runtime/bin/node" --import="$loader" "$script"

# A runtime with the component resolves the engine the component carries.
installed="$work/with-component"
rm -rf "$installed"
engine="$installed/components/versions/1.0.0/runtime/node_modules/@deepseek-ai/libreoffice-kit"
mkdir -p "$engine/lib"
printf '{"version":"1.0.0"}\n' > "$installed/components/current.json"
cat > "$engine/lib/index.js" <<'ENGINE'
export function createConverter() {
  return Promise.resolve({ render: async () => ({ missingFonts: [] }) })
}
ENGINE
DSH_HOME="$installed" "$tree/runtime/bin/node" --import="$loader" "$script"
