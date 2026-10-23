#!/usr/bin/env bash
# Stage the existing native binary and packaged Linux Host; never build or download.
# Usage: package-appimage.sh [--appdir-only]
# Requires the native release binary and apps/desktop/.desktop-build/targets/linux-x64.
# Without appimagetool on PATH (or with --appdir-only), retain only the AppDir.
set -euo pipefail
appdir_only=false
case "${1:-}" in
  '') [ "$#" -eq 0 ] || { echo "package-appimage: unexpected empty argument" >&2; exit 2; } ;;
  --appdir-only) appdir_only=true; shift ;;
  *) echo "usage: $0 [--appdir-only]" >&2; exit 2 ;;
esac
[ "$#" -eq 0 ] || { echo "usage: $0 [--appdir-only]" >&2; exit 2; }
root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
bin="$root/apps/desktop-tauri/native-app/target/release/dsh-desktop-native-app"
tree="$root/apps/desktop/.desktop-build/targets/linux-x64"
icon="$root/apps/desktop-tauri/src-tauri/icons/icon.png"
host="$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host"
for required in "$bin" "$tree/bin/desktop-host" "$tree/runtime/bin/node" \
  "$tree/runtime/primary-runtime/dependencies/node/bin/node"; do
  [ -x "$required" ] || { echo "package-appimage: missing executable: $required; prepare the Linux target and native release binary first" >&2; exit 1; }
done
for required in "$host/package.json" "$host/lib/tauri.js" "$host/native/native-loader.mjs" \
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  "$tree/project/package.json" "$tree/project/cordis.yml" "$tree/project/cordis.patch.yml" "$icon"; do
  [ -f "$required" ] || { echo "package-appimage: missing packaged input: $required" >&2; exit 1; }
done
# Match the bundled Host version, not an unrelated native Cargo package version.
version=$("$tree/runtime/bin/node" --input-type=module -e \
  'import { readFileSync } from "node:fs"; const { version } = JSON.parse(readFileSync(process.argv[1], "utf8")); if (typeof version !== "string" || !/^[0-9A-Za-z.+-]+$/.test(version)) throw new Error("invalid packaged Host version"); console.log(version)' \
  "$host/package.json")
work=$(mktemp -d "${TMPDIR:-/tmp}/dsh-native-appimage.XXXXXX")
appdir="$work/DeepSeek-Harness.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/dsh" "$appdir/usr/share/applications" \
  "$appdir/usr/share/icons/hicolor/1024x1024/apps"
cp "$bin" "$appdir/usr/bin/dsh-desktop-native"
# The Host launcher resolves ../runtime/bin/node; preserve symlinks and modes.
for part in bin dsh runtime project; do
  cp -a "$tree/$part" "$appdir/usr/share/dsh/$part"
done
printf '%s\n' "$version" > "$appdir/usr/share/dsh/version"
cp "$icon" "$appdir/dsh-desktop-native.png"
cp "$icon" "$appdir/.DirIcon"
cp "$icon" "$appdir/usr/share/icons/hicolor/1024x1024/apps/dsh-desktop-native.png"
cat > "$appdir/dsh-desktop-native.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=DeepSeek Harness (native)
Exec=dsh-desktop-native
Icon=dsh-desktop-native
Categories=Development;
Terminal=false
DESKTOP
cp "$appdir/dsh-desktop-native.desktop" "$appdir/usr/share/applications/"
cat > "$appdir/AppRun" <<'APPRUN'
#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root="$here/usr/share/dsh"
# The AppImage is read-only; profile edits belong in the user's DSH home.
export DSH_HOME="${DSH_HOME:-$HOME/.dsh}"
profile="$DSH_HOME/desktop-project"
if [ ! -d "$profile" ]; then
  mkdir -p "$DSH_HOME"
  cp -a "$root/project" "$profile"
  chmod -R u+w "$profile"
fi
export DSH_DESKTOP_HOST_BIN="$root/bin/desktop-host"
export DSH_DESKTOP_RUNTIME_DIR="$root/dsh"
export DSH_DESKTOP_PROJECT_DIR="$profile"
export DSH_DESKTOP_PRIMARY_RUNTIME="$root/runtime/primary-runtime"
export DSH_DESKTOP_PNPM_ENTRY="$root/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs"
export DSH_DESKTOP_NODE_BIN="$root/runtime/bin"
version=$(cat "$root/version")
export DSH_DESKTOP_VERSION="$version"
export DSH_CLIENT_VERSION="$version"
exec "$here/usr/bin/dsh-desktop-native" "$@"
APPRUN
chmod +x "$appdir/AppRun" "$appdir/usr/bin/dsh-desktop-native"
echo "AppDir prepared at $appdir"
if [ "$appdir_only" = false ] && command -v appimagetool >/dev/null 2>&1; then
  ARCH=x86_64 appimagetool "$appdir" "$root/apps/desktop-tauri/native-app/dsh-desktop-native.AppImage"
  echo "built: $root/apps/desktop-tauri/native-app/dsh-desktop-native.AppImage"
else
  echo "AppImage not built (--appdir-only or appimagetool not on PATH)."
fi
