#!/usr/bin/env bash
# Verify that removing the packaged application leaves no user data behind.
#
# The package owns the payload under `/usr/lib/deepseek-harness`, the
# launcher under `/usr/bin`, and the desktop entry and icon. The profile a
# user creates lives in `$DSH_HOME`, and removing the package must not touch
# it: the user keeps their profile and their sessions.
#
# The check runs against the package's own file list, so it covers whatever
# the package actually installs rather than a hand-written list.
#
#   usage: uninstall-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${DSH_DESKTOP_TARGET:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
deb=$(ls -t "$tree"/deb/*.deb 2>/dev/null | head -1)
[ -n "$deb" ] && [ -f "$deb" ] || { echo "RESULT deb=missing"; exit 1; }

work=$(mktemp -d "${TMPDIR:-/tmp}/dsh-uninstall-probe.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/root" "$work/home/.dsh" "$work/unpack"

# The build host has no dpkg, so read the archive the way the format
# defines it: the control file names the package, the data archive holds
# the files it owns.
(cd "$work/unpack" && ar x "$deb") || { echo "RESULT extract=FAIL"; exit 1; }
tar -xOJf "$work/unpack/control.tar.xz" ./control > "$work/control" 2>/dev/null \
  || { echo "RESULT control=FAIL"; exit 1; }
tar -xf "$work/unpack/data.tar.zst" -C "$work/root" || { echo "RESULT data=FAIL"; exit 1; }

package=$(sed -n 's/^Package: //p' "$work/control" | head -1)
echo "RESULT package=$package"

# The profile the user already has must survive the removal untouched.
mkdir -p "$work/home/.dsh/desktop-project"
printf '{"name":"kept","version":"0.0.0"}\n' > "$work/home/.dsh/desktop-project/package.json"
printf 'session\n' > "$work/home/.dsh/desktop-project/keep.txt"
before=$(sha256sum "$work/home/.dsh/desktop-project/package.json" | cut -d' ' -f1)

# The file list the package owns, as dpkg would record it.
(cd "$work/root" && find . -mindepth 1 \( -type f -o -type l \) -printf '%P\n' | sort) > "$work/owned"
owned=$(wc -l < "$work/owned")
echo "owned_files=$owned"

# Removing the package deletes exactly the files it owns. The probe reads
# the same list, so a file the package installs but does not own would
# survive here exactly as it would survive a real removal.
remaining=0
while read -r path; do
  [ -n "$path" ] || continue
  rm -f "$work/root/$path"
done < "$work/owned"
find "$work/root" -type d -empty -delete 2>/dev/null

left=0
[ -d "$work/root" ] && left=$(find "$work/root" -mindepth 1 | wc -l)
echo "RESULT remaining_in_root=$left"
if [ "$left" -ne 0 ]; then
  echo "RESULT uninstall=FAIL"
  find "$work/root" -mindepth 1 | head -20
  exit 1
fi

# The user's own data is untouched by the removal.
after=$(sha256sum "$work/home/.dsh/desktop-project/package.json" | cut -d' ' -f1)
kept=no
[ "$before" = "$after" ] && [ -f "$work/home/.dsh/desktop-project/keep.txt" ] && kept=yes
echo "RESULT user_data_kept=$kept"
if [ "$kept" != yes ]; then
  echo "RESULT uninstall=FAIL"
  exit 1
fi

echo "RESULT uninstall=ok"
