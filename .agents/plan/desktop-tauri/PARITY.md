# Parity checklist — native desktop vs web client

Native (egui) must reach parity with the web client for every screen below.
A screen is done only when it runs on the native app and its probe passes.

## Core
- [ ] Session list (M2) ✓
- [ ] Session viewer / log (M2) ✓
- [ ] Chat: prompt, stream, cancel, tool calls (M3) ✓ (transport) — UI pending
- [ ] Permission approval (M4) — typing done; Remote Event channel pending
- [ ] Model selection (M4) ✓
- [ ] Session management: search/create/rename/fork (M4) ✓

## Tools & terminal
- [ ] Terminal create/list/rename/close/attach (M5) ✓ (transport) — UI pending

## Settings & plugins
- [ ] Settings describe/update/replace/mutate (M6) ✓ (transport)
- [ ] Plugin inventory (M6) ✓
- [ ] Skills list (M6) ✓
- [ ] Schedules list/catalog/history/delete/update (M6) ✓
- [ ] Workspace files read/stat/list (M6) ✓

## Still missing for full parity
- [ ] Document preview
- [ ] Marketplace / plugin manager UI
- [ ] Deliverables view
- [ ] Feedback / human interaction
- [ ] Goals, jobs, todo, plan panels
- [ ] Remote Event waterfall (approval responses, projections)
- [ ] Host sidecar launch + authentication in the native app
- [ ] Packaging: AppImage, updater, tray, single-instance, dialog

## Memory gate
- [ ] RSS < 300 MiB measured on a real machine (native floor estimate 240–290 MiB)
