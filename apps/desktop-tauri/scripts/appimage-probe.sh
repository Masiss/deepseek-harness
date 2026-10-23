#!/usr/bin/env bash
# Verify one packed AppImage without touching the desktop session.
#
# Two checks, both headless:
#   A. AppRun's environment contract and first-run profile seeding, with a stub
#      standing in for the GUI shell.
#   B. The packaged Host boots from a read-only extraction, serves the bridge,
#      and shuts down on the control frame.
#
# Isolation: `env -i` with private HOME/TMPDIR/DSH_HOME/XDG dirs, no
# DISPLAY/WAYLAND_DISPLAY, `setsid` so the whole run has its own process group,
# and extraction instead of FUSE. Nothing is written outside the work dir.
#
#   usage: appimage-probe.sh [path-to.AppImage]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${DSH_DESKTOP_TARGET:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
appimage=${1:-$(ls -t "$tree"/appimage/*.AppImage 2>/dev/null | head -1)}
[ -n "$appimage" ] && [ -f "$appimage" ] || { echo "RESULT appimage=missing"; exit 1; }

work="$tree/appimage/probe"
# The previous run made its extraction read-only.
chmod -R u+w "$work" 2>/dev/null
rm -rf "$work"
mkdir -p "$work/root" "$work/home" "$work/tmp" "$work/cache" "$work/config"
cd "$work/root" || exit 1
"$appimage" --appimage-extract > /dev/null 2>&1 || { echo "RESULT extract=FAIL"; exit 1; }
extract="$work/root/squashfs-root"
echo "RESULT appimage=$(basename "$appimage") bytes=$(stat -c %s "$appimage")"

# --- A. AppRun contract, with a stub shell binary -------------------------
cp -a "$extract" "$work/apprun"
shell_bin=$(basename "$(find "$work/apprun/usr/bin" -maxdepth 1 -name 'dsh-desktop-*' | head -1)")
[ -n "$shell_bin" ] || { echo "RESULT shell=missing"; exit 1; }
cat > "$work/apprun/usr/bin/$shell_bin" <<'STUB'
#!/bin/sh
echo "shell=$0"
echo "appimage=${APPIMAGE:-unset}"
echo "host_bin=$DSH_DESKTOP_HOST_BIN"
echo "runtime_dir=$DSH_DESKTOP_RUNTIME_DIR"
echo "project_dir=$DSH_DESKTOP_PROJECT_DIR"
echo "version=$DSH_DESKTOP_VERSION"
echo "client_version=$DSH_CLIENT_VERSION"
echo "project_seeded=$([ -f "$DSH_DESKTOP_PROJECT_DIR/package.json" ] && echo yes || echo no)"
echo "node_exec=$([ -x "$DSH_DESKTOP_RUNTIME_DIR/../runtime/bin/node" ] && echo yes || echo no)"
STUB
chmod +x "$work/apprun/usr/bin/$shell_bin"
env -i PATH=/usr/bin:/bin HOME="$work/home" TMPDIR="$work/tmp" \
  XDG_CACHE_HOME="$work/cache" XDG_CONFIG_HOME="$work/config" \
  DSH_HOME="$work/home/.dsh" APPIMAGE="$appimage" APPDIR="$work/apprun" \
  "$work/apprun/AppRun"
echo "seeded_in_home=$(find "$work/home/.dsh/desktop-project" -maxdepth 1 -name package.json 2>/dev/null | wc -l)"
echo "seed_writable=$([ -w "$work/home/.dsh/desktop-project/package.json" ] && echo yes || echo no)"

# --- B. Packaged Host from a read-only extraction -------------------------
chmod -R a-w "$extract"
root="$extract/usr/share/dsh"
home="$work/home/.dsh"
profile="$home/desktop-project"
mkdir -p "$home"
cp -a "$root/project" "$profile"
mkfifo "$work/in"

cat > "$work/run.sh" <<EOF
#!/bin/sh
echo \$\$ > "$work/pid"
exec env -i PATH=/usr/bin:/bin HOME="$work/home" TMPDIR="$work/tmp" \
  XDG_CACHE_HOME="$work/cache" XDG_CONFIG_HOME="$work/config" DSH_HOME="$home" \
  DSH_DESKTOP_RUNTIME_DIR="$root/dsh" DSH_DESKTOP_PROJECT_DIR="$profile" \
  DSH_DESKTOP_PRIMARY_RUNTIME="$root/runtime/primary-runtime" \
  DSH_DESKTOP_PNPM_ENTRY="$root/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  DSH_DESKTOP_NODE_BIN="$root/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 \
  "$root/bin/desktop-host" "$root/dsh" "$profile" \
  "$root/runtime/primary-runtime" \
  "$root/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$root/runtime/bin"
EOF
chmod +x "$work/run.sh"
setsid "$work/run.sh" < "$work/in" > "$work/out" 2> "$work/err" &
sleep 0.5
pgid=$(cat "$work/pid" 2>/dev/null || true)
exec 3> "$work/in"

url=""
for _ in $(seq 1 120); do
  url=$(grep -o 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$work/out" 2>/dev/null | head -1)
  [ -n "$url" ] && break
  sleep 0.5
done

if [ -z "$url" ]; then
  echo "RESULT host=FAIL"
  tail -5 "$work/err"
  [ -n "$pgid" ] && kill -KILL -- -"$pgid" 2>/dev/null
  exit 1
fi

port=$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')
base="http://127.0.0.1:$port"
echo "RESULT host=OK port=$port"
curl -s -L -c "$work/cookies" -b "$work/cookies" -o /dev/null "$url"
echo "bridge_status=$(curl -s -m 20 -b "$work/cookies" -H 'content-type: application/json' \
  -X POST "$base/__dsh-desktop/updates-status" -d null)"
echo "image_writable_files=$(find "$extract" -type f -perm -u+w 2>/dev/null | wc -l)"

printf '@@DSH@@{"type":"shutdown"}\n' >&3
exec 3>&-
for _ in $(seq 1 20); do
  kill -0 "$pgid" 2>/dev/null || break
  sleep 0.5
done
kill -KILL -- -"$pgid" 2>/dev/null
echo "RESULT done=1"
