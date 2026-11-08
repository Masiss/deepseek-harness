# QA and acceptance — v1.2

## Visual authority

The unchanged `reference/dsh-ui-prototype-v2.html` is the accepted composition and menu interaction reference. Do not treat screenshots of the former v1.1 recreated shell as a pixel baseline. Keep shell 30/34/26, menu and workspace navigation, Explorer/Agent/Changes ownership.

## Native GPUI gates

- Capture actual shipped GPUI at 1366×768 and 1920×1080, plus a narrow viewport; verify 100% and 125% UI scale.
- Inspect app menu open/close, workspace picker, session picker, status icons and Preferences. Test click and keyboard (Up/Down/Enter/Esc), restore focus and IME in composer.
- Test tool and approval states (idle/running/awaiting decision/failed/reconnect) with real Host and stream metadata; no demo placeholders in shipping UI.
- Verify Git Diff/Files/Branches and scope of Stage, Unstage, Discard, checkout and commit; stale-guard and explicit destructive confirmation.
- Verify hide/show Explorer/Changes and terminal doesn't cancel running agent/PTY; test logical wrap paste vs intentional newlines.
- Use pixel screenshots with a real GUI environment and manual or automated visual review; geometry-only `debug_bounds` is not proof of visual fidelity.

## Preview QA scope for this artifact

- Static V2 HTML `openMainMenu()` binding, original popup markup and settings picker verified by source inspection.
- Version 1.2 design guide embeds this unchanged V2 via `srcdoc` instead of recreating product shell.
- JavaScript syntax + HTML structure should be statically checked before handoff.
- Browser end-to-end validation requires a non-policy-blocked runtime; do not claim Playwright visual PASS if the local browser refuses URLs.

## Report format

Changed files; before/after pixel evidence; commands used; interaction state verified; measured layout; source/Host capability; `SUPPORTED/PARTIAL/NOT_SUPPORTED`; remaining risks and explicit user-visible mismatches.
