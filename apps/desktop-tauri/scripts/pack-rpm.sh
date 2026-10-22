#!/usr/bin/env bash
# Pack the prepared Linux tree into an RPM with Fedora's own rpmbuild.
#
# The host has no RPM tooling, so the build runs in a pinned Fedora container
# that installs `rpm-build` and reads the staged payload from a mounted
# directory. The spec keeps dependency discovery off: the bundle contains a
# Node runtime and thousands of modules, and the package's real requirements
# are GTK 3 and WebKitGTK 4.1, which the shell links directly.
#
#   usage: pack-rpm.sh [linux-x64|linux-arm64]
set -eu

target=${1:-linux-$(uname -m | sed 's/x86_64/x64/; s/aarch64/arm64/')}
case "$target" in
  linux-x64) rpm_arch=x86_64 ;;
  linux-arm64) rpm_arch=aarch64 ;;
  *) echo "pack-rpm: unsupported target $target" >&2; exit 2 ;;
esac

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
tree="$repo/apps/desktop/.desktop-build/targets/$target"
work="$tree/rpm"
stage="$work/stage"
full=$(node -p "require('$repo/package.json').version")
version=${full%%-*}
release=${full#*-}
if [ "$release" = "$full" ]; then release=1; fi

for required in "$tree/bin/desktop-host" "$tree/project/package.json" \
  "$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/package.json" \
  "$tree/runtime/bin/node"; do
  [ -e "$required" ] || { echo "pack-rpm: missing $required; run prepare-linux-runtime first" >&2; exit 1; }
done

echo "pack-rpm: building the shell"
cargo build --release --manifest-path "$repo/apps/desktop-tauri/src-tauri/Cargo.toml"

rm -rf "$stage"
. "$(dirname -- "$0")/stage-payload.sh"
stage_payload "$stage" "$tree" "$full" \
  "$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri" \
  "$repo/apps/desktop-tauri/src-tauri/icons/icon.png"

cat > "$work/deepseek-harness.spec" <<EOF
%global __os_install_post %{nil}
%global debug_package %{nil}
%define _binary_payload w19.zstdio

Name: deepseek-harness
Version: $version
Release: $release
Summary: DeepSeek Harness desktop application
License: MIT
URL: https://github.com/deepseek-ai/deepseek-harness
AutoReqProv: no
Requires: gtk3
Requires: webkit2gtk4.1
Recommends: libayatana-appindicator-gtk3

%description
An agent harness desktop application with the Host, runtime, and a seed
profile packaged together. User data lives in \$DSH_HOME.

%install
mkdir -p %{buildroot}
cp -a /rpm/stage/. %{buildroot}/

%post
# The desktop entry carries the `dsh://` scheme handler.
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database || true
fi

%files
%defattr(-,root,root,-)
%attr(0755,root,root) /usr/bin/deepseek-harness
/usr/lib/deepseek-harness
/usr/share/applications/deepseek-harness.desktop
/usr/share/icons/hicolor/1024x1024/apps/deepseek-harness.png
EOF

# The build container decides the packed RPM's bytes, so the image is pinned
# by digest rather than by a moving tag.
fedora_image="fedora:41@sha256:f1a3fab47bcb3c3ddf3135d5ee7ba8b7b25f2e809a47440936212a3a50957f3d"
echo "pack-rpm: building in fedora:41 ($fedora_image)"
docker run --rm -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" -v "$work:/rpm" "$fedora_image" sh -c '
  set -e
  # The previous build may have left root-owned artifacts behind.
  rm -rf /rpm/work /rpm/rpmbuild.log /rpm/*.rpm
  dnf install -y -q rpm-build > /dev/null
  rpmbuild --define "_topdir /rpm/work" -bb /rpm/deepseek-harness.spec > /rpm/rpmbuild.log 2>&1 \
    || { tail -40 /rpm/rpmbuild.log; exit 1; }
  cp /rpm/work/RPMS/*/*.rpm /rpm/
  # The container builds as root; hand everything back to the invoking user.
  chown -R "$HOST_UID:$HOST_GID" /rpm
'
rpmfile=$(ls -t "$work"/*.rpm | head -1)
echo "pack-rpm: $(stat -c %s "$rpmfile") bytes at $rpmfile"
