#!/usr/bin/env bash
# Run one command with no reach into the desktop session.
#
# The sandbox owns a pid namespace (no other process is visible or signalable),
# an empty /run (no Wayland, D-Bus, or compositor sockets), a private /tmp, no
# controlling terminal, and a cleared environment. The rest of the filesystem is
# read-only; only $DSH_ISOLATION_DIR is writable and becomes the working
# directory. Loopback networking stays shared because the update probes serve
# their feed locally.
#
#   usage: DSH_ISOLATION_DIR=/path/to/work isolated-run.sh <command> [args...]
set -eu

work=${DSH_ISOLATION_DIR:?set DSH_ISOLATION_DIR to an existing writable directory}
[ -d "$work" ] || { echo "isolated-run: $work does not exist" >&2; exit 2; }
mkdir -p "$work/home" "$work/tmp"

# Probe inputs that name read-only paths survive the clear; everything else the
# caller exported stays out of the sandbox.
forward=()
for name in DSH_OFFICE_ROOT DSH_OFFICE_COMPONENTS DSH_DESKTOP_TARGET NODE_OPTIONS MALLOC_ARENA_MAX FLAGS ARENA; do
  value=${!name:-}
  if [ -n "$value" ]; then forward+=(--setenv "$name" "$value"); fi
done

exec bwrap \
  --unshare-pid --unshare-uts --unshare-ipc --unshare-cgroup-try \
  --new-session --die-with-parent \
  --ro-bind / / \
  --dev /dev --proc /proc \
  --tmpfs /tmp --tmpfs /run \
  --bind "$work" "$work" \
  --chdir "$work" \
  --clearenv \
  --setenv PATH /usr/bin:/bin \
  --setenv HOME "$work/home" \
  --setenv TMPDIR "$work/tmp" \
  --setenv DSH_ISOLATION_DIR "$work" \
  "${forward[@]}" \
  -- "$@"
