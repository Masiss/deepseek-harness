#!/usr/bin/env bash
# Shared helpers for the desktop probes.
#
# A probe owns the processes it starts and nothing else: the helpers here
# never look for a process by name, because a name matches the user's own
# application too.

# Stop a process and every process it started.
#
# @param root - pid of the process to stop.
kill_tree() {
  local root=${1:-}
  [ -n "$root" ] || return 0
  python3 - "$root" <<'PY'
import os, signal, subprocess, sys
root = int(sys.argv[1])
rows = {}
for line in subprocess.check_output(['ps', '-eo', 'pid=,ppid='], text=True).splitlines():
    pid, ppid = line.split()
    rows.setdefault(int(ppid), []).append(int(pid))
stack = [root]
seen = set()
while stack:
    pid = stack.pop()
    if pid in seen:
        continue
    seen.add(pid)
    for child in rows.get(pid, []):
        stack.append(child)
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
PY
}

# Wait until the webview reports the session state it sees.
#
# The shell asks the Host from the page itself, with the cookie the browser
# holds, so the answer belongs to the webview and not to the probe.
#
# @param file - file holding the shell's standard output.
# @param timeout - seconds to wait.
# @returns the reported state, or "none".
wait_for_webview_auth() {
  local file=$1 timeout=${2:-60}
  local deadline=$((SECONDS + timeout))
  local state=none
  while [ "$SECONDS" -lt "$deadline" ]; do
    state=$(grep -ho 'dsh: webview-auth=[A-Za-z0-9]*' "$file" 2>/dev/null | tail -1)
    case "$state" in
      *200) break ;;
      none) ;;
      *) break ;;
    esac
    sleep 0.2
  done
  printf '%s' "$state"
}

# Count the established connections to a local port.
#
# @param port - port the Host listens on.
# @returns the number of established sockets.
established_connections() {
  local port=$1
  python3 - "$port" <<'PY'
import sys
port = int(sys.argv[1])
count = 0
for path in ('/proc/net/tcp', '/proc/net/tcp6'):
    try:
        rows = open(path).read().splitlines()[1:]
    except OSError:
        continue
    for row in rows:
        fields = row.split()
        if len(fields) < 4:
            continue
        local = fields[1]
        state = fields[3]
        if state != '01':
            continue
        try:
            local_port = int(local.rsplit(':', 1)[1], 16)
        except (IndexError, ValueError):
            continue
        if local_port == port:
            count += 1
print(count)
PY
}
