# Design decisions v1.2

## Frozen decisions — approved DSH V2

- Agent-first native desktop. Explorer on the **left**, Agent in the **center**, Changes on the **right**.
- Topbar **30px**, central Agent tab **34px**, statusbar **26px**.
- `☰` opens **App Menu**, workspace selector opens recent workspaces separately, search opens Quick Open / palette through their own commands.
- Explorer has **no title row** and no Activity Bar; Session `+` and picker live at the **bottom of Explorer**. No permanent thread navigator.
- Changes has its **own right-panel header** and Diff/Files tabs; no duplicate Changes buffer in Agent center.
- Bottom-left Git/branch/sync; bottom-right terminal/diagnostics/connection. No Settings shortcut or duplicate Terminal toggle.
- Preferences is one window with General, Appearance, Keymap, Extensions, Skills, Marketplace, About.
- Side panels independently resize/toggle; when too narrow, hide panels instead of making the Agent unreadable.
- Keep original V2 prototype untouched in `reference/dsh-ui-prototype-v2.html`. Any design evolution must be explicitly approved and regression-tested.

## V1.2 amendments

1. The v1.1 demo shell invented by the design-guide was **not** the visual authority. It was removed and replaced with V2 embedded live.
2. The incorrectly wired hamburger (`☰ → Command Palette`) was removed. Original V2 binds `☰ → openMainMenu()`.
3. Zed docs are a reference for navigation quality, panel behavior and keyboard patterns, *not* an authority to replace DSH's approved layout or invent IDE features.
4. `tokens.json` and 3×3 theme/accent demos are *design proposals*. Current Host capabilities in the 2026-11-06 map are different; no required feature is supported solely because the prototype shows it.
5. No UI-driven fake state or optimistic destructive completion. All production success must follow Host ACK.
6. The handoff is a **design reference**; native GPUI has not been visually re-verified in this artifact release.

See `HAMBURGER_AND_NAVIGATION.md`, `DSH_FEATURE_TRACEABILITY.md`, `ZED_ALIGNMENT.md` and the frozen V2 reference.
