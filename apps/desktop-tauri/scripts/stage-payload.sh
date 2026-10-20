#!/bin/sh
# Stage the Linux payload both system packages share.
#
# The .deb and .rpm carry one tree: the Host, the dsh runtime, the primary
# runtime (Node, pnpm, no Office payload), a seed profile, and one launcher in
# /usr/bin that derives the payload from its own resolved path. Office is an
# on-demand component, so no package carries it.
#
#   usage: . stage-payload.sh; stage_payload <stage> <tree> <version> <shell> <icon>
set -eu

stage_payload() {
  local stage=$1 tree=$2 version=$3 shell=$4 icon=$5 part

  for part in bin dsh runtime project; do
    [ -e "$tree/$part" ] || { echo "stage-payload: missing $tree/$part" >&2; exit 1; }
  done

  mkdir -p "$stage/usr/bin" "$stage/usr/lib/deepseek-harness" \
    "$stage/usr/share/applications" "$stage/usr/share/icons/hicolor/1024x1024/apps"
  for part in bin dsh runtime project; do
    cp -a "$tree/$part" "$stage/usr/lib/deepseek-harness/$part"
  done
  cp "$shell" "$stage/usr/lib/deepseek-harness/dsh-desktop-tauri"
  cp "$icon" "$stage/usr/share/icons/hicolor/1024x1024/apps/deepseek-harness.png"

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
# The Office component is a separate download from the same release feed.
if [ -n "${DSH_COMPONENT_FEED:-}" ]; then
  export DSH_DESKTOP_COMPONENT_FEED="$DSH_COMPONENT_FEED"
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
}
