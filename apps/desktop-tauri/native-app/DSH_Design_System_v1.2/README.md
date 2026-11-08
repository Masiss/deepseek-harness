# DSH Design System v1.2 — V2-fidelity / Zed-informed

**Status:** Design and agent handoff; **not** an implemented GPUI UI or verified against the latest DSH repository. Updated 2026-11-08.

## Sources and precedence

1. `../DSH_UI_V2_Approved_Reference.html` — **approved, untouched visual + interactive reference**, including the actual hamburger, workspace/session pickers, panel geometry and menus.
2. `../DSH_UI_Handoff_v1.0/DSH_UI_Functional_Spec_v1.0.md` — product behavior contract, subject to live Host capability review.
3. `../DSH_UI_Handoff_v1.0/runtime-capability-map.md` — **historical snapshot 2026-11-06** of GPUI + Host; useful for scope, not proof of today's code.
4. `../DSH_UI_Handoff_v1.0/DSH_UI_Implementation_Backlog_v1.0.md` — historical implementation progress.
5. `HAMBURGER_AND_NAVIGATION.md`, `DSH_FEATURE_TRACEABILITY.md`, `ZED_ALIGNMENT.md` — v1.2 delta and UX review criteria.
6. `FOUNDATIONS.md`, `COMPONENTS.md`, `PATTERNS.md`, `SCREENS.md` — supplementary system definitions. Where these diverge from V2 or Host, **do not treat them as authority**.

## Preview files

- `design-system.html` — token/component guide with V2 original **embedded live** in an iframe. Theme and accent switches on the design-system page apply to the surrounding guide; use V2's own Preferences to change the embedded app.
- `../DSH_UI_V2_Approved_Reference.html` — run standalone for pixel and interaction comparison. This prototype uses sample data and does not execute Host operations.
- `tokens.json` — proposed semantic token layer; the shipped implementation resolves theme roles from `src/theme/packs/*.json` instead, so no `tokens.json` exists yet.
- `prototype-git-v1.3.html` — extension of the frozen V2 file with the Git review surface: grouped staging, hunk actions, commit/amend/push, a commit-graph History tab, and conflict banner. Same sample-data rules as V2; the approved reference stays unchanged. See `GIT_SURFACE_V1.3.md` for the surface inventory, Host verb mapping, and verification evidence, and `IMPLEMENTATION_PLAN.md` for slices G0–G4.

## What changed from v1.1

- **Reverted the demo's invented shell** and restored the *actual approved V2* as the preview. The shell used to bind `☰` to Command Palette; V2 binds it to App Menu.
- Removed the extra desktop-specimen toolbar from above the app.
- Added explicit contracts for app menu, workspace picker, search/command palette, session picker, and Settings.
- Added **DSH-specific** capability-to-surface traceability with historical confidence markers and distinctions from Zed.
- Re-prioritized coding-agent work: verify source and native screenshots before adding UI features.

## No silent redesign

- Keep chrome dimensions top 30px / Agent tab 34px / status 26px, the three-panel layout, no Activity Bar, no separate settings icon or terminal toggle, no persistent session list.
- Preserve Explorer at left, Agent center, Changes right as specified by DSH V2 even though Zed supports a different modern Agentic arrangement.
- Real implementation must use Host-confirmed state, and not claim `connected`, `installed`, `successful`, `saved`, `approved` without acknowledgement.
- Never introduce unsupported Ask/Build agent modes, permanent approvals, verified plugin badges or a full editor/LSP to fill empty space.

## Handoff order

Read `AGENT_PROMPT.md`. Start with D0: observe native GPUI and map features to real code. Then D1: fix **hamburger/navigation parity** and clear visual mismatches. Then D2+: address missing DSH workflows one slice at a time. Native screenshot at 1366×768 and 1920×1080 is mandatory before visual DONE; geometry tests alone are insufficient.
