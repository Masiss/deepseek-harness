#!/usr/bin/env bash
# Run one command with no reach into the desktop session, inside a budget.
#
# The sandbox owns a pid namespace (no other process is visible or
# signalable), an empty /run (no Wayland, D-Bus, or compositor sockets), a
# private /tmp, no controlling terminal, and a cleared environment. The rest
# of the filesystem is read-only; only $DSH_ISOLATION_DIR is writable and
# becomes the working directory. Loopback networking stays shared because the
# update probes serve their feed locally.
#
# The budgets bound what a runaway probe can take from the machine:
#
# - CPU: the cgroup's quota, so a busy loop is throttled rather than
#   monopolising a core.
# - RAM: the cgroup's memory limit, with swap off, so an allocation over the
#   limit is killed instead of being paged out.
# - Disk: the writable directory and the private /tmp are temporary
#   filesystems sized to the budget, so a write past it fails.
# - Processes: the cgroup's process limit, so a fork bomb stops.
#
# The budgets come from the environment, with defaults a probe can live
# inside. Each one is reported, so a run says which budget it ran under and
# which one the machine could not provide.
#
#   usage: DSH_ISOLATION_DIR=/path/to/work isolated-run.sh <command> [args...]
#
#   DSH_ISOLATION_CPU     CPU quota, as a percentage of one core (default 200%)
#   DSH_ISOLATION_MEMORY  RAM limit, in bytes with a size suffix (default 2G)
#   DSH_ISOLATION_DISK    writable disk budget (default 2G)
#   DSH_ISOLATION_PIDS    process limit (default 512)
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

# A size with a unit suffix, as the filesystem and the cgroup take it.
size_to_bytes() {
  local value=$1 number unit
  case "$value" in
    *[Kk]*) number=${value%[Kk]}; unit=1024 ;;
    *[Mm]*) number=${value%[Mm]}; unit=$((1024 * 1024)) ;;
    *[Gg]*) number=${value%[Gg]}; unit=$((1024 * 1024 * 1024)) ;;
    *[Tt]*) number=${value%[Tt]}; unit=$((1024 * 1024 * 1024 * 1024)) ;;
    *) number=$value; unit=1 ;;
  esac
  echo $((number * unit))
}

cpu=${DSH_ISOLATION_CPU:-200%}
memory=${DSH_ISOLATION_MEMORY:-2G}
disk=${DSH_ISOLATION_DISK:-2G}
pids=${DSH_ISOLATION_PIDS:-512}

# The cgroup carries the CPU, RAM, and process budgets. It is the only
# mechanism that bounds a whole process tree, so the sandbox joins it before
# it starts.
cgroup_root=""
cgroup=""
if [ -d /sys/fs/cgroup ] && [ "$(stat -fc %T /sys/fs/cgroup 2>/dev/null || true)" = cgroup2fs ]; then
  # The delegated subtree is the one the user's own service owns; a process
  # may only limit what it may write to.
  for candidate in "/sys/fs/cgroup/user.slice/user-$(id -u).slice/user@$(id -u).service" \
    "/sys/fs/cgroup/user.slice/user-$(id -u).slice" \
    "/sys/fs/cgroup"; do
    if [ -w "$candidate/cgroup.subtree_control" ] && [ -w "$candidate/cgroup.procs" ]; then
      cgroup_root=$candidate
      break
    fi
  done
fi

if [ -n "$cgroup_root" ]; then
  cgroup="$cgroup_root/dsh-isolation-$$"
  if mkdir -p "$cgroup" 2>/dev/null; then
    echo "+cpu +memory +pids" > "$cgroup_root/cgroup.subtree_control" 2>/dev/null || true
    # The quota is a fraction of a core; the memory limit has swap off so an
    # over-budget allocation is killed rather than paged out.
    period=100000
    quota=$(awk -v percent="${cpu%%%}" -v period="$period" 'BEGIN { printf "%d", (percent * period) / 100 }')
    echo "$quota $period" > "$cgroup/cpu.max" 2>/dev/null || true
    echo "$memory" > "$cgroup/memory.max" 2>/dev/null || true
    echo 0 > "$cgroup/memory.swap.max" 2>/dev/null || true
    echo "$pids" > "$cgroup/pids.max" 2>/dev/null || true
    # Joining the cgroup here means the sandbox and everything it starts is
    # inside the budget.
    if ! echo $$ > "$cgroup/cgroup.procs" 2>/dev/null; then
      cgroup=""
    fi
  else
    cgroup=""
  fi
fi

if [ -n "$cgroup" ]; then
  echo "isolated-run: budget cpu=$cpu memory=$memory disk=$disk pids=$pids"
else
  # Without a cgroup the process tree is unbounded; the disk budget still
  # applies, and the run says so.
  echo "isolated-run: budget cpu=unavailable memory=unavailable disk=$disk pids=unavailable" >&2
fi

disk_bytes=$(size_to_bytes "$disk")

# The writable directory and the private /tmp are temporary filesystems
# sized to the disk budget, so a write past it fails instead of filling the
# machine.
bwrap_args=(
  --unshare-pid --unshare-uts --unshare-ipc --unshare-cgroup-try
  --new-session --die-with-parent
  --ro-bind / /
  --dev /dev --proc /proc
  --size "$disk_bytes" --tmpfs /tmp
  --tmpfs /run
  --size "$disk_bytes" --tmpfs "$work"
  --dir "$work/home" --dir "$work/tmp"
  --chdir "$work"
  --clearenv
  --setenv PATH /usr/bin:/bin
  --setenv HOME "$work/home"
  --setenv TMPDIR "$work/tmp"
  --setenv DSH_ISOLATION_DIR "$work"
)

# The scratch directory the sandbox writes to has to exist inside the
# temporary filesystem the budget sizes.
mkdir -p "$work/home" "$work/tmp"

exec bwrap \
  "${bwrap_args[@]}" \
  "${forward[@]}" \
  -- "$@"
