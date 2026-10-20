#!/usr/bin/env bash
# Pack the prepared Linux tree into a Debian package.
#
# Layout: the payload lives in `/usr/lib/deepseek-harness`, the shell in
# `usr/lib/deepseek-harness/dsh-desktop-tauri`, and `/usr/bin/deepseek-harness`
# derives both from its own path, so the same tree boots from a real install or
# from a `dpkg-deb -x` extraction. The profile is user state and is seeded into
# `$DSH_HOME` on first run, exactly as the AppImage does.
#
#   usage: pack-deb.sh [linux-x64|linux-arm64]
set -eu

target=${1:-linux-$(uname -m | sed 's/x86_64/x64/; s/aarch64/arm64/')}
case "$target" in
  linux-x64) deb_arch=amd64 ;;
  linux-arm64) deb_arch=arm64 ;;
  *) echo "pack-deb: unsupported target $target" >&2; exit 2 ;;
esac

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
tree="$repo/apps/desktop/.desktop-build/targets/$target"
work="$tree/deb"
stage="$work/DeepSeek-Harness"
version=$(node -p "require('$repo/package.json').version")
out="$work/deepseek-harness_${version}_${deb_arch}.deb"

for required in "$tree/bin/desktop-host" "$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/package.json" \
  "$tree/runtime/bin/node" "$tree/project/cordis.yml"; do
  [ -e "$required" ] || { echo "pack-deb: missing $required; run prepare-linux-runtime first" >&2; exit 1; }
done

echo "pack-deb: building the shell"
cargo build --release --manifest-path "$repo/apps/desktop-tauri/src-tauri/Cargo.toml"

rm -rf "$stage"
mkdir -p "$stage/DEBIAN" "$stage/usr/bin" "$stage/usr/lib/deepseek-harness" \
  "$stage/usr/share/applications" "$stage/usr/share/icons/hicolor/1024x1024/apps"

for part in bin dsh runtime project; do
  cp -a "$tree/$part" "$stage/usr/lib/deepseek-harness/$part"
done
cp "$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri" "$stage/usr/lib/deepseek-harness/"
cp "$repo/apps/desktop-tauri/src-tauri/icons/icon.png" \
  "$stage/usr/share/icons/hicolor/1024x1024/apps/deepseek-harness.png"

cat > "$stage/usr/bin/deepseek-harness" <<'EOF'
#!/bin/sh
# Resolve the payload from this launcher, seed the profile once, then start the
# shell with the environment the Host contract expects.
set -e
self=$(readlink -f "$0")
root=$(dirname "$(dirname "$self")")/lib/deepseek-harness

home=${DSH_HOME:-$HOME/.dsh}
profile="$home/desktop-project"
if [ ! -d "$profile" ]; then
  mkdir -p "$home"
  cp -a "$root/project" "$profile"
  # Keep the seeded profile writable even if the packaged copy is not.
  chmod -R u+w "$profile"
fi

export DSH_DESKTOP_HOST_BIN="$root/bin/desktop-host"
export DSH_DESKTOP_RUNTIME_DIR="$root/dsh"
export DSH_DESKTOP_PROJECT_DIR="$profile"
export DSH_DESKTOP_PRIMARY_RUNTIME="$root/runtime/primary-runtime"
export DSH_DESKTOP_PNPM_ENTRY="$root/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs"
export DSH_DESKTOP_NODE_BIN="$root/runtime/bin"
export DSH_DESKTOP_VERSION="${DSH_DESKTOP_VERSION:-__VERSION__}"
export DSH_CLIENT_VERSION="${DSH_CLIENT_VERSION:-__VERSION__}"
if [ -n "${DSH_APPIMAGE_UPDATE_FEED:-}" ]; then
  export DSH_DESKTOP_UPDATE_FEED="$DSH_APPIMAGE_UPDATE_FEED"
fi
exec "$root/dsh-desktop-tauri" "$@"
EOF
sed -i "s/__VERSION__/$version/g" "$stage/usr/bin/deepseek-harness"
chmod +x "$stage/usr/bin/deepseek-harness" "$stage/usr/lib/deepseek-harness/dsh-desktop-tauri"

cat > "$stage/usr/share/applications/deepseek-harness.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=DeepSeek Harness
Comment=Agent harness desktop application
Exec=deepseek-harness %U
Icon=deepseek-harness
Categories=Development;
Terminal=false
MimeType=x-scheme-handler/dsh;
EOF

installed_size=$(du -sk "$stage/usr" | cut -f1)
cat > "$stage/DEBIAN/control" <<EOF
Package: deepseek-harness
Version: $version
Section: devel
Priority: optional
Architecture: $deb_arch
Installed-Size: $installed_size
Maintainer: DeepSeek Harness fork <noreply@localhost>
Depends: libgtk-3-0, libwebkit2gtk-4.1-0
Recommends: libayatana-appindicator3-1
Description: DeepSeek Harness desktop application
 An agent harness desktop application with the Host, runtime, and a seed
 profile packaged together. User data lives in \$DSH_HOME.
EOF

# The build host has no dpkg (this fork builds on Arch), so assemble the
# ar archive the Debian format defines: debian-binary, control, then data.
echo "pack-deb: packing $out"
rm -f "$out" "$work/debian-binary" "$work/control.tar.xz" "$work/data.tar.zst"
printf '2.0\n' > "$work/debian-binary"
tar -C "$stage/DEBIAN" --owner=0 --group=0 -cJf "$work/control.tar.xz" .
tar -C "$stage" --owner=0 --group=0 --exclude=./DEBIAN -cf - . \
  | zstd -19 -T0 --long=27 -q -o "$work/data.tar.zst"
(cd "$work" && ar rc "$out" debian-binary control.tar.xz data.tar.zst)
echo "pack-deb: $(stat -c %s "$out") bytes at $out"
