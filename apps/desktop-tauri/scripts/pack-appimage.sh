#!/usr/bin/env bash
# Pack the Tauri shell and the prepared Linux runtime tree into one AppImage.
#
# The image is read-only, so the seed profile ships inside it and the first run
# copies it into the writable DSH home; plugins then install there, never into
# the image. `AppRun` exports the same positional contract the development and
# packaged-tree probes use, so all three launch paths share one Host entry.
#
#   usage: pack-appimage.sh [linux-x64|linux-arm64]
set -eu

target=${1:-linux-$(uname -m | sed 's/x86_64/x64/; s/aarch64/arm64/')}
case "$target" in
  linux-x64) appimage_arch=x86_64 ;;
  linux-arm64) appimage_arch=aarch64 ;;
  *) echo "pack-appimage: unsupported target $target" >&2; exit 2 ;;
esac

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
tree="$repo/apps/desktop/.desktop-build/targets/$target"
work="$tree/appimage"
appdir="$work/DeepSeek-Harness.AppDir"
version=$(node -p "require('$repo/package.json').version")
out="$work/DeepSeek-Harness-$version-$appimage_arch.AppImage"

for required in "$tree/bin/desktop-host" "$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/package.json" \
  "$tree/runtime/bin/node" "$tree/runtime/primary-runtime/dependencies/node/bin/node" "$tree/project/package.json"; do
  [ -e "$required" ] || { echo "pack-appimage: missing $required; run prepare-linux-runtime first" >&2; exit 1; }
done

echo "pack-appimage: building the shell"
cargo build --release --manifest-path "$repo/apps/desktop-tauri/src-tauri/Cargo.toml"

rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/dsh" "$appdir/usr/share/applications"

cp "$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri" "$appdir/usr/bin/"
for part in bin dsh runtime project; do
  cp -a "$tree/$part" "$appdir/usr/share/dsh/$part"
done

cp "$repo/apps/desktop-tauri/src-tauri/icons/icon.png" "$appdir/deepseek-harness.png"
cp "$appdir/deepseek-harness.png" "$appdir/.DirIcon"

cat > "$appdir/deepseek-harness.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=DeepSeek Harness
Comment=Agent harness desktop application
Exec=dsh-desktop-tauri %U
Icon=deepseek-harness
Categories=Development;
Terminal=false
MimeType=x-scheme-handler/dsh;
EOF
cp "$appdir/deepseek-harness.desktop" "$appdir/usr/share/applications/"

cat > "$appdir/AppRun" <<EOF
#!/bin/sh
# Resolve the payload beside this script, then hand the shell the same
# environment the packaged-tree probes use.
set -e
here=\$(CDPATH= cd -- "\$(dirname -- "\$0")" && pwd)
root="\$here/usr/share/dsh"

# The image is read-only. The profile is user state: seed it once, then leave
# every profile write (plugins, patches) to the copy under the DSH home.
home=\${DSH_HOME:-\$HOME/.dsh}
profile="\$home/desktop-project"
if [ ! -d "\$profile" ]; then
  mkdir -p "\$home"
  cp -a "\$root/project" "\$profile"
  # The image is read-only; a copy that kept those bits would refuse every
  # plugin install later, so the seeded profile is explicitly user-writable.
  chmod -R u+w "\$profile"
fi

export DSH_DESKTOP_HOST_BIN="\$root/bin/desktop-host"
export DSH_DESKTOP_RUNTIME_DIR="\$root/dsh"
export DSH_DESKTOP_PROJECT_DIR="\$profile"
export DSH_DESKTOP_PRIMARY_RUNTIME="\$root/runtime/primary-runtime"
export DSH_DESKTOP_PNPM_ENTRY="\$root/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs"
export DSH_DESKTOP_NODE_BIN="\$root/runtime/bin"
# The installed version drives the update comparison; a release without one
# simply has no update source. The product telemetry config reads
# DSH_CLIENT_VERSION, and a Host that booted without it silently drops that
# entry, so both names carry the same release identity.
export DSH_DESKTOP_VERSION="\${DSH_DESKTOP_VERSION:-$version}"
export DSH_CLIENT_VERSION="\${DSH_CLIENT_VERSION:-$version}"
if [ -n "\${DSH_APPIMAGE_UPDATE_FEED:-}" ]; then
  export DSH_DESKTOP_UPDATE_FEED="\$DSH_APPIMAGE_UPDATE_FEED"
fi
# The Office component is a separate download from the same release feed.
if [ -n "\${DSH_COMPONENT_FEED:-}" ]; then
  export DSH_DESKTOP_COMPONENT_FEED="\$DSH_COMPONENT_FEED"
fi
exec "\$here/usr/bin/dsh-desktop-tauri" "\$@"
EOF
chmod +x "$appdir/AppRun" "$appdir/usr/bin/dsh-desktop-tauri"

# Pinned release: the `continuous` build's desktop-file check waits on a child
# with WUNTRACED and reports a valid file as broken, so it never packs here.
tool="$HOME/.cache/dsh-appimage/appimagetool-1.9.1-$appimage_arch.AppImage"
if [ ! -x "$tool" ]; then
  mkdir -p "$(dirname "$tool")"
  echo "pack-appimage: fetching appimagetool 1.9.1"
  curl -fsSL -o "$tool" \
    "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-$appimage_arch.AppImage"
  chmod +x "$tool"
fi

echo "pack-appimage: packing $out"
rm -f "$out"
ARCH="$appimage_arch" "$tool" "$appdir" "$out"

echo "pack-appimage: $(stat -c %s "$out") bytes at $out"
