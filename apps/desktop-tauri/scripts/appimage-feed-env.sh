#!/bin/sh
# Emit shell-safe, baked feed defaults; explicit runtime settings take precedence.
set -eu
quote() {
  printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}
for name in DSH_DESKTOP_UPDATE_FEED DSH_DESKTOP_COMPONENT_FEED; do
  case "$name" in
    DSH_DESKTOP_UPDATE_FEED) value=${DSH_APPIMAGE_UPDATE_FEED:-} ;;
    DSH_DESKTOP_COMPONENT_FEED) value=${DSH_COMPONENT_FEED:-} ;;
  esac
  printf 'if [ -z "${%s:-}" ]; then export %s=' "$name" "$name"
  quote "$value"
  printf '; fi\n'
done
