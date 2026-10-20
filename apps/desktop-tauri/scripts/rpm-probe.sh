#!/usr/bin/env bash
# Verify a packed RPM on a clean Fedora container.
#
# Installs from the local file, checks the file list and integrity, exercises
# the launcher contract with a stub standing in for the GUI shell, and boots the
# packaged Host headlessly. Nothing is installed on the build host.
#
#   usage: rpm-probe.sh [path-to.rpm]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${DSH_DESKTOP_TARGET:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
rpm=${1:-$(ls -t "$tree"/rpm/*.rpm 2>/dev/null | head -1)}
[ -n "$rpm" ] && [ -f "$rpm" ] || { echo "RESULT rpm=missing"; exit 1; }
work="$tree/rpm/probe"
mkdir -p "$work"
cp "$rpm" "$work/"

cat > "$work/probe.sh" <<'PROBE'
set -e
dnf install -y -q /pkg/*.rpm > /tmp/dnf.log 2>&1 || { tail -30 /tmp/dnf.log; exit 1; }
echo "RESULT install=OK"
echo "RESULT package=$(rpm -q deepseek-harness)"
echo "RESULT files=$(rpm -ql deepseek-harness | wc -l)"
rpm -V deepseek-harness > /tmp/verify.log 2>&1 && echo "RESULT verify=OK" || { echo "RESULT verify=FAIL"; cat /tmp/verify.log; }

cat > /usr/lib/deepseek-harness/dsh-desktop-tauri <<'STUB'
#!/bin/sh
echo "launcher_ok host=$DSH_DESKTOP_HOST_BIN"
echo "project=$DSH_DESKTOP_PROJECT_DIR version=$DSH_DESKTOP_VERSION client=$DSH_CLIENT_VERSION"
echo "manifest=$([ -f "$DSH_DESKTOP_PROJECT_DIR/package.json" ] && echo yes || echo no) node=$([ -x "$DSH_DESKTOP_NODE_BIN/node" ] && echo yes || echo no)"
STUB
chmod +x /usr/lib/deepseek-harness/dsh-desktop-tauri
DSH_HOME=/tmp/dsh-home /usr/bin/deepseek-harness
echo "RESULT seeded=$(find /tmp/dsh-home/desktop-project -maxdepth 1 -name package.json | wc -l) size=$(du -sh /usr/lib/deepseek-harness | cut -f1)"

mkdir -p /tmp/host-home
cp -a /usr/lib/deepseek-harness/project /tmp/host-home/desktop-project
timeout 30 env -i PATH=/usr/bin:/bin HOME=/tmp/host-home DSH_HOME=/tmp/host-home \
  DSH_DESKTOP_RUNTIME_DIR=/usr/lib/deepseek-harness/dsh \
  DSH_DESKTOP_PROJECT_DIR=/tmp/host-home/desktop-project \
  DSH_DESKTOP_PRIMARY_RUNTIME=/usr/lib/deepseek-harness/runtime/primary-runtime \
  DSH_DESKTOP_PNPM_ENTRY=/usr/lib/deepseek-harness/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs \
  DSH_DESKTOP_NODE_BIN=/usr/lib/deepseek-harness/runtime/bin DSH_DESKTOP_VERSION=1.0.0 DSH_CLIENT_VERSION=1.0.0 \
  /usr/lib/deepseek-harness/bin/desktop-host /usr/lib/deepseek-harness/dsh /tmp/host-home/desktop-project \
  /usr/lib/deepseek-harness/runtime/primary-runtime \
  /usr/lib/deepseek-harness/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs \
  /usr/lib/deepseek-harness/runtime/bin > /tmp/host.out 2> /tmp/host.err || true
if grep -q 'http://127.0.0.1' /tmp/host.out /tmp/host.err; then echo "RESULT host=OK"; else echo "RESULT host=FAIL"; tail -5 /tmp/host.err; fi
PROBE

echo "RESULT rpm=$(basename "$rpm") bytes=$(stat -c %s "$rpm")"
docker run --rm -v "$work:/pkg:ro" "fedora:41" bash /pkg/probe.sh
