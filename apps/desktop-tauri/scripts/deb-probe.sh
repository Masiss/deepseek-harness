#!/usr/bin/env bash
# Verify a packed .deb without installing it and without touching the session.
#
# A. The launcher's environment contract and first-run profile seeding, with a
#    stub standing in for the GUI shell (so no window opens).
# B. The packaged Host boots from a read-only extraction and serves the bridge.
#
# Run it under isolated-run.sh: DSH_ISOLATION_DIR must be writable.
#
#   usage: DSH_ISOLATION_DIR=... isolated-run.sh bash deb-probe.sh [deb]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${DSH_DESKTOP_TARGET:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
deb=${1:-$(ls -t "$tree"/deb/*.deb 2>/dev/null | head -1)}
[ -n "$deb" ] && [ -f "$deb" ] || { echo "RESULT deb=missing"; exit 1; }

work=${DSH_ISOLATION_DIR:?set DSH_ISOLATION_DIR}
root="$work/root"
home="$work/home/.dsh"
rm -rf "$root" "$work/home"
mkdir -p "$root" "$work/home" "$work/tmp" "$work/unpack"
# No dpkg on the build host: parse the same ar archive it would.
(cd "$work/unpack" && ar x "$deb" && tar -xf data.tar.* -C "$root") \
  || { echo "RESULT extract=FAIL"; exit 1; }

echo "RESULT deb=$(basename "$deb") bytes=$(stat -c %s "$deb")"
echo "installed_size=$(du -sk "$root" | cut -f1)KiB"
tar -xOJf "$work/unpack/control.tar.xz" ./control | sed -n '/Package:/,/Description:/p' | head -8

# --- A. launcher contract with a stub shell -------------------------------
cat > "$root/usr/lib/deepseek-harness/dsh-desktop-tauri" <<'STUB'
#!/bin/sh
echo "host_bin=$DSH_DESKTOP_HOST_BIN"
echo "project_dir=$DSH_DESKTOP_PROJECT_DIR"
echo "version=$DSH_DESKTOP_VERSION"
echo "client_version=$DSH_CLIENT_VERSION"
echo "project_has_cordis=$([ -f "$DSH_DESKTOP_PROJECT_DIR/cordis.yml" ] && echo yes || echo no)"
echo "node_exec=$([ -x "$DSH_DESKTOP_NODE_BIN/node" ] && echo yes || echo no)"
STUB
chmod +x "$root/usr/lib/deepseek-harness/dsh-desktop-tauri"
env -i PATH=/usr/bin:/bin HOME="$work/home" TMPDIR="$work/tmp" DSH_HOME="$home" \
  "$root/usr/bin/deepseek-harness"
echo "seeded=$(find "$home/desktop-project" -maxdepth 1 -name cordis.yml 2>/dev/null | wc -l)"
echo "seed_writable=$([ -w "$home/desktop-project/cordis.yml" ] && echo yes || echo no)"
echo "seed_outside_root=$([ -d "$root/usr/lib/deepseek-harness/project" ] && echo ok || echo missing)"

# --- B. packaged Host from a read-only extraction -------------------------
payload="$root/usr/lib/deepseek-harness"
chmod -R a-w "$root"
mkdir -p "$home"
cp -a "$payload/project" "$home/desktop-project"
mkfifo "$work/in"

cat > "$work/run.sh" <<EOF
#!/bin/sh
echo \$\$ > "$work/pid"
exec env -i PATH=/usr/bin:/bin HOME="$work/home" TMPDIR="$work/tmp" DSH_HOME="$home" \\
  DSH_DESKTOP_RUNTIME_DIR="$payload/dsh" DSH_DESKTOP_PROJECT_DIR="$home/desktop-project" \\
  DSH_DESKTOP_PRIMARY_RUNTIME="$payload/runtime/primary-runtime" \\
  DSH_DESKTOP_PNPM_ENTRY="$payload/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \\
  DSH_DESKTOP_NODE_BIN="$payload/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 \\
  DSH_CLIENT_VERSION=1.0.0 \\
  "$payload/bin/desktop-host" "$payload/dsh" "$home/desktop-project" \\
  "$payload/runtime/primary-runtime" \\
  "$payload/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$payload/runtime/bin"
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
  chmod -R u+w "$root"
  exit 1
fi

port=$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')
base="http://127.0.0.1:$port"
echo "RESULT host=OK port=$port"
curl -s -L -c "$work/cookies" -b "$work/cookies" -o /dev/null "$url"
echo "bridge_status=$(curl -s -m 20 -b "$work/cookies" -H 'content-type: application/json' \
  -X POST "$base/__dsh-desktop/updates-status" -d null)"
echo "image_writable_files=$(find "$root" -type f -perm -u+w 2>/dev/null | wc -l)"

printf '@@DSH@@{"type":"shutdown"}\n' >&3
exec 3>&-
for _ in $(seq 1 20); do
  kill -0 "$pgid" 2>/dev/null || break
  sleep 0.5
done
kill -KILL -- -"$pgid" 2>/dev/null
chmod -R u+w "$root"
echo "RESULT done=1"
