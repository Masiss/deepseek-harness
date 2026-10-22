#!/usr/bin/env bash
# Package the native egui app as an AppImage (best-effort; no updater).
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
bin="$root/apps/desktop-tauri/native-app/target/release/dsh-desktop-native-app"
[ -x "$bin" ] || { echo "build the binary first"; exit 1; }
work=$(mktemp -d /tmp/dsh-appimage.XXXXXX)
appdir="$work/DeepSeek-Harness.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/256x256/apps"
cp "$bin" "$appdir/usr/bin/dsh-desktop-native"
cat > "$appdir/usr/share/applications/dsh-desktop-native.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=DeepSeek Harness (native)
Exec=dsh-desktop-native
Icon=dsh-desktop-native
Categories=Development;
DESKTOP
# AppRun
cat > "$appdir/AppRun" <<'APPRUN'
#!/bin/sh
exec "$(dirname "$0")/usr/bin/dsh-desktop-native" "$@"
APPRUN
chmod +x "$appdir/AppRun"
if command -v appimagetool >/dev/null 2>&1; then
  appimagetool "$appdir" "$root/apps/desktop-tauri/native-app/dsh-desktop-native.AppImage"
  echo "built: $root/apps/desktop-tauri/native-app/dsh-desktop-native.AppImage"
else
  echo "appimagetool not found; AppDir prepared at $appdir"
fi
