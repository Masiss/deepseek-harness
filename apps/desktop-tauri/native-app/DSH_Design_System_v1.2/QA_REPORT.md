# v1.2 artifact QA — 2026-11-08

## Scope

Validated the **HTML design reference**, not the native Rust/GPUI build.
The Browser plugin was unavailable. Direct Playwright navigation to local URLs was blocked by Chromium enterprise `URLBlocklist`; **Playwright `set_content()`** worked for offline HTML smoke tests. Screenshots and interactions below were produced with Chromium 1366×768, 1920×1080, and the design guide at 1440×900.

## Passed

- Original V2 copied unchanged into `reference/dsh-ui-prototype-v2.html` and the standalone preview.
- `design-system.html` embeds that exact original V2 content in `iframe.srcdoc`; no rewritten product shell remains.
- Original V2 `☰` opens an anchored menu with Workspace, Customize and About groups; **not** command palette.
- Appearance menu item opens the consolidated Preferences UI with Appearance selected; Esc closes.
- Workspace picker, Session picker and search icon open distinct surfaces; Esc dismisses.
- Guide iframe `☰` also opens the correct original menu. Browser page errors: none in tested flows.
- JS source syntax validated using `node --check` on guide and original V2.
- Measured original V2 shell (1366×768): topbar 30px, Agent tab 34px, statusbar 26px, Explorer width 258px, Changes width 485px.
- Original V2 at 1920×1080: topbar 30px / Agent tab 34px; native-size screenshot captured.
- Existing v1.1 shell screenshots removed from v1.2 to avoid false visual references.

## Known limits / issues to audit in native code

- Reference V2 is a **sample-data visual prototype**; popup keyboard/ARIA semantics, Open Folder and production states require Host-backed integration. Do not claim original V2 satisfies all `HAMBURGER_AND_NAVIGATION.md` acceptance items.
- Current pinned GPUI native build and latest source were not accessible in this turn. No claims of native UI visual regression PASS.
- Browser screenshot pixel comparisons are between HTML variants; native GPUI pixel snapshots remain a separate engineering task.
- Zed concepts are documented in `ZED_ALIGNMENT.md`; no assumption that Zed's current layout or APIs are DSH's implementation.

## Evidence files

`reference-v2-menu-open.png`, `reference-v2-preferences.png`, `preview-v12-menu-open.png`, `reference-v2-1366x768.png`, `reference-v2-1920x1080.png`.
