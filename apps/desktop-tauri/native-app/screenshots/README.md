# Native UI screenshots

Captured from the release binary of this checkout (disconnected/empty state, no Host session):

- `light-1366x768.png`, `light-1440x900.png`, `light-1920x1080.png`
- `dark-1366x768.png`, `dark-1440x900.png`, `dark-1920x1080.png`

The theme is selected through `native-ui-preferences.json` (`{"theme":"light"}` or `{"theme":"dark"}`)
and the app runs against an isolated `DSH_HOME`.

## Capture recipe (headless)

GPUI 0.2.2 panics on a Wayland compositor without a seat, so the app runs as an X11 client under
Xwayland and the window is read directly:

```sh
weston --backend=headless-backend.so --renderer=pixman --width=2200 --height=1400 \
  --xwayland --no-config &
export DISPLAY=:1 XDG_RUNTIME_DIR=/tmp/<run> VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
DSH_HOME=/tmp/<home> HOME=/tmp/<home> ./target/release/dsh-desktop-native-app &
xdotool search --onlyvisible --name '.*'        # pick the 1440×900 window
xdotool windowsize <id> 1366 768
magick import -window <id> out.png
```

Input cannot be injected without a seat, so interactive states (menus, approvals, History detail)
need a real graphical session.
