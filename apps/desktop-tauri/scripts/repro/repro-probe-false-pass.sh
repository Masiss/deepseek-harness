#!/usr/bin/env bash
# Reproduce the cold-start probe's false PASS.
#
# The probe reports `cold_start=OK` when the webview finished loading pages,
# and checks the Host bridge with curl and a cookie jar of its own. A Host
# that refuses the webview's requests therefore still produces a passing
# probe, because the page it loads finishes loading either way.
#
# The stub Host answers every request with the same 401 the real Host sends
# an unauthenticated browser, and hands the shell a ready frame.
#
#   usage: repro-probe-false-pass.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
work=$(mktemp -d /tmp/dsh-repro-false-pass.XXXXXX)

cat > "$work/stub-host.py" <<'STUB'
import http.server, json, sys

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = b'dsh web authentication required; reopen the URL printed by dsh web.\n'
        self.send_response(401)
        self.send_header('content-type', 'text/plain; charset=utf-8')
        self.send_header('content-length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass

server = http.server.HTTPServer(('127.0.0.1', 0), Handler)
url = f'http://127.0.0.1:{server.server_address[1]}/?token=stub-token'
sys.stderr.write(f'dsh web: {url}\n')
sys.stderr.flush()
sys.stdout.write('@@DSH@@' + json.dumps({'type': 'ready', 'url': url}) + '\n')
sys.stdout.flush()
server.serve_forever()
STUB

cat > "$work/stub-host" <<SH
#!/usr/bin/env bash
exec python3 "$work/stub-host.py"
SH
chmod +x "$work/stub-host"

# The probe launches the packaged Host; the stub replaces it.
probe="$work/cold-start-probe.sh"
sed -e "s#^repo=.*#repo=$repo#" \
  -e "s#DSH_DESKTOP_HOST_BIN=\"\$tree/bin/desktop-host\"#DSH_DESKTOP_HOST_BIN=\"$work/stub-host\"#" \
  "$repo/apps/desktop-tauri/scripts/cold-start-probe.sh" > "$probe"
grep -q "$work/stub-host" "$probe" || { echo 'RESULT repro=FAIL reason=probe-patch'; exit 1; }

output=$(DSH_COLD_START_WORK="$work/cold" bash "$probe" "$target" 2>&1)
echo "$output" | grep -E '^RESULT|^MILESTONE' | head -12

if echo "$output" | grep -q 'RESULT cold_start=OK'; then
  echo "RESULT repro=false-pass CONFIRMED: the probe reports OK while the webview is unauthorized"
  echo "$output" | grep -E '^RESULT bridge=' | head -2
  exit 0
fi
echo 'RESULT repro=false-pass REFUTED: the probe failed on an unauthorized webview'
exit 1
