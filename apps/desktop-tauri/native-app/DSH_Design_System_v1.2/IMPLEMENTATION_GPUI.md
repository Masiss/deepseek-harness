> **V1.2 source precedence:** The untouched approved `reference/dsh-ui-prototype-v2.html` owns visuals and `DSH_FEATURE_TRACEABILITY.md` plus live Host evidence own capability claims. This document is supplementary; it cannot override the frozen V2 layout or claim a feature exists.

# GPUI implementation playbook

This plan intentionally avoids prescribing unverified APIs for the DSH codebase. Start by inspecting actual `Cargo.toml`, `src/gpui_main.rs`, `src/shell/`, `ui_preferences.rs`, action registry, client-core bridge and Host RPC contracts.

## Layers

```text
src/shell/design_system/
  theme.rs          # DsTheme / semantic roles; token mapping
  metrics.rs        # density, scale, layout bounds
  typography.rs     # text roles, line height, font resolution
  actions.rs        # shared keyboard/focus conventions
  primitives/       # Button, IconButton, Field, Popover, Menu, Dialog...
  composites/       # SessionPicker, ToolActivityGroup, DiffHunk...
  showcase/         # optional native story/gallery, debug builds only

src/shell/screens/   # application view composition using DS components
src/shell/adapters/  # existing UI ↔ client-core/Host mapping
```

Names above are architectural suggestions, not file existence claims. Factor incrementally; avoid a mass mechanical rewrite or changing runtime functionality to accommodate styling.

## Theme implementation

- `DsTheme` with semantic role getters (`canvas`, `text`, `accent`, `primary_fill`, `border`, `danger_surface` etc.). Theme `Dark/Midnight/Light`; accent `Blue/Teal/Violet`; density `Compact/Standard/Comfortable`; UI scale user preference.
- Resolve one **snapshot** of theme/metrics per window render; no ad-hoc HEX at feature call sites. Materialized `tokens.json` is external reference; do not hot-read JSON on each render.
- Keep persisted theme values versioned. Unknown theme/accent from older preferences falls back to Dark/Blue/Standard with optional migration notice; never panic.
- Changes to theme should update all active windows/overlays; focus and user draft retained.

## Behavior implementation

- Use typed actions / existing registry for shortcuts and menu labels. Avoid parallel shortcut registration systems. Focus ownership and return targets must be explicit in GPUI (the web won't supply native keyboard defaults).
- Build layer-specific primitives to minimize duplication: base button/input/dialog/menu; then DSH-specific composites. Do not fork similar versions inside screens.
- Separate `ViewState` (expanded rows, hover, scroll, selected tab) from `RuntimeState` (Host-confirmed run, Git, approval, plugin install); route mutations through existing adapter with request correlation and outcome.
- All async callbacks need cancellation or stale-owner protection: closing/switching sessions cannot let late responses mutate the wrong screen.
- Preserve scroll per owner (transcript vs tree vs diff vs terminal), not one global scroll container.

## Evaluate external GPUI libraries — do not assume drop-in

- `gpui-ce/gpui-component` / GPUI Kit has reusable styled components, `gpui-base` has more neutral behavior and test helpers; useful for studying interaction and test patterns.
- Its current releases may pin another GPUI generation. DSH's archived handoff names GPUI 0.2.2. Before adoption, prove dependency compatibility via isolated branch `cargo tree`, minimal compile fixture, size/perf diff, license check and conflict with existing window/render APIs.
- Prefer *conceptual reuse* (focus, menu, accessibility, virtualization, test ideas) if upgrades risk large product regressions.
- GPUI remains pre-1.0; check the exact currently pinned version rather than copying code from newer docs blindly.

## Native UI verification tiers

1. **Pure:** Rust unit tests for theme token resolution, state transitions, reducer logic, capability gating, copy-wrap and settings persistence.
2. **In-process:** existing GPUI test-support if available for focus, click, keyboard and layout. Do not claim test-support exists in a pinned version without compiling.
3. **Rendered:** screenshot diff from actual app (stable fonts, DPI, locale, device scale, time/cursor masked). Include dark/light and multi-state golden images.
4. **Packaged desktop:** exercise exact Host+PTY+Git+approval flows under X11/Wayland if product supports them. Linux tools (Xvfb/Weston, screenshot, input automation) are optional and isolated. Headless/virtual screenshot cannot prove packaged accessibility.
5. **Review:** compare against frozen V2 baseline *and* 1.1 approved deltas, judge keyboard, density, contrast and recovery, not pixels alone.

## Change boundaries

**Allowed by default:** design-system code, component composition, local UI preferences, tests, screenshots, documentation.

**Require separately agreed scope:** Host RPC changes, database schema, agent policies, security grants, session lifecycle redesign, dependency upgrade of GPUI, large refactor of terminal or Git engine. When capability missing, mark `NOT_SUPPORTED`, explain and defer rather than implementing an invented UI completion state.

## Risky details to specifically test

- UI color and layout update after theme/accent/scale switch; no stale popup theme.
- IME composition: Enter confirms candidate, does not submit draft.
- Long commands/URLs: visual wrap vs raw copy payload (newlines preserved only if present in source text).
- Splitter drag and keyboard focus ring remain visible across clipping.
- Approval idempotency after reconnect; no duplicated approve operation.
- Terminal: Ctrl+C semantics, bracketed paste, split resize, reconnect/resume.
- Model picker and attachment state continue through session switch/reconnect.
- Git stale diff / dirty checkout / terminal command failures do not display false success.
