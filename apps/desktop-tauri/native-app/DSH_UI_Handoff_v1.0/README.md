# DSH UI Handoff v1.0

English | [中文](README.zh.md)

Handoff set for the native Desktop interface: the behavior spec, the runtime capability map, the implementation backlog, and one HTML file as the visual reference.

## Contents

| File | Role | Authority |
|---|---|---|
| [`DSH_UI_Functional_Spec_v1.0.md`](DSH_UI_Functional_Spec_v1.0.md) | Behavior contract: feature ids, acceptance criteria, roadmap R0–R5 | Authority for intended *behavior* |
| [`runtime-capability-map.md`](runtime-capability-map.md) | Maps each requirement to the Host RPCs and the gpui shell that exist | Authority for *implementation state* (audited date inside) |
| [`DSH_UI_Implementation_Backlog_v1.0.md`](DSH_UI_Implementation_Backlog_v1.0.md) | Work order, acceptance gates, report format | Authority for *work order* |
| [`dsh-ui-prototype-v2.html`](dsh-ui-prototype-v2.html) | Self-contained HTML: layout, color tokens, panels, states | Authority for *visuals* |

## Relationship to the rest of `native-app`

- `../src/` — the shipped application: **Rust + gpui** (`src/gpui_main.rs`, `src/shell/`). The egui shell (`src/main.rs`) was deleted on 2026-11-06.
- `../prototype/` — the older interactive prototype (`index.html` + `docs/`) that describes the egui-era interface; still useful for flows, but not the visual reference for this handoff.
- `DSH_UI_Handoff_v1.0/` — visual and behavioral reference for the gpui port.
- `../USER_GUIDE.md` and `../USER_FLOWS.md` — describe the running application; when this set and the application disagree, the application is the truth and this set is the target.

When a behavior in the spec has no Host capability, record `NOT_SUPPORTED` with an honest fallback. Never build a handler that fakes a successful state.

## How to use it

1. Read `runtime-capability-map.md` first to learn what can be wired today and what needs Host work.
2. Take a task from the backlog, one vertical slice at a time (T01, T02, …).
3. Report per backlog section 3: changed files, capabilities used, remaining `NOT_SUPPORTED`, tests run, risks.

## Maintenance

The audited date lives in `runtime-capability-map.md`; after a Host RPC or gpui shell change, update the matching table and that date. The V2 HTML is frozen for visual regression, not edited alongside code.
