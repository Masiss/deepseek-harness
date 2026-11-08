# DSH hamburger, navigation and menus — authoritative v1.2 contract

## IA from approved V2 (not a generic dashboard)

| Control | Location | Opens / does | Explicitly must NOT |
|---|---|---|---|
| `☰` | top-left, first item; 30px topbar | **Application Menu** anchored to this control | Open Command Palette or toggle Explorer directly |
| Workspace name `▾` | immediately right of `☰` | Recent workspaces + Open Folder (spec WS-01), selected workspace | Open App Menu / change Session |
| Search icon | top-right | Quick Open / command surface per command registry; distinguish Quick Open `Ctrl+P` from Command Palette `Ctrl+Shift+P` | Duplicate workspace picker |
| Changes toggle | top-right | Show/hide existing right dock | Create a Changes tab in Agent center |
| `+` / session `▾` | *foot of Explorer* | New Session / current Session picker, including search, recent/pin/archive as supported | Permanent threads sidebar; extra topbar session picker |
| Branch/Git/sync | status-left | Repository state and Git/Changes entry | Duplicate full Git panel |
| Terminal / diagnostics / connection | status-right | Terminal dock and diagnostics status | Settings icon, fake connected indicator |

## Exact App Menu (V2 visual grouping)

```
☰ App Menu
  Workspace
    Quick Open                 Ctrl+P (user-configurable)
    Toggle Explorer            [current binding]
    New Session                [current binding]
  ───────────────────────────────────
  Customize
    Preferences…               Ctrl+, (user-configurable)
    Appearance
    Keymap
    Extensions
    Skills
    Marketplace
  ───────────────────────────────────
    About DSH
```

Each `Customize` row opens the **same Preferences window** at a different selected section. Menu text/shortcut must derive from the **single command registry**; never maintain two command implementations. Add entries only with an owner/capability decision, not because Zed has them. Appearance/Keymap/etc are navigation shortcuts, not separate settings apps.

## Interaction contract

1. Click `☰`: show anchored menu, `aria-expanded=true`; keyboard Up/Down/Home/End and Enter/Space operate items; Esc and outside click close and restore focus.
2. Click workspace label: **different** anchored picker. It lists recent workspaces with current selection; Open Folder must invoke the genuine folder flow; Host failure does not mutate active selection.
3. Ctrl+P = Quick Open (files/paths); Ctrl+Shift+P = Command Palette (commands). Search button chooses a documented one — does not overload hamburger.
4. Only one popover at a time. Reopening its anchor toggles closed. Overlays restore focus and do not trap keyboard under them.
5. Menu actions are disabled with reason when their capability is unavailable; don't render a successful toast as a substitute for Host execution.
6. Selecting an item with a submenu opens that submenu with keyboard, or opens Preferences at the named tab as specified; no hover-only required action.
7. Labels/paths used in the V2 mock are sample data. Native GPUI must provide real selection, session state and Git state.

## Visual acceptance

Topbar 30px with menu icon aligned to its 35px hit region, then workspace selector. No bulky rounded CTA buttons. Anchored compact menu with short section captions, separator strokes, hover/selected/focus states, shortcut column. No icons-heavy Activity Bar or permanent Preferences side button. Test popup near viewport edge at 1366×768 and on narrow windows. In light/dark, text contrast remains sufficient.

## Correction to earlier preview

`v1.1 design-system.html: $('appMenu').onclick = () => overlay('paletteOverlay')` was **wrong**. In v1.2 visual preview, the original V2 `openMainMenu()` is used unchanged. The actual GPUI implementation still requires code review.
