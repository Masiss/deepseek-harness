> **V1.2 source precedence:** The untouched approved `reference/dsh-ui-prototype-v2.html` owns visuals and `DSH_FEATURE_TRACEABILITY.md` plus live Host evidence own capability claims. This document is supplementary; it cannot override the frozen V2 layout or claim a feature exists.

# Foundations — tokens, themes, layout, typography

The shipped numeric source of truth is `src/theme/packs/*.json` (one JSON document per pack); metrics live in `src/layout.rs` and the resolved `Theme` snapshot. There is no `tokens.json` yet: resolve semantic tokens from the app-owned theme context and never scatter hex or magic padding constants through screens.

## Color and token model

```text
primitive palette  →  semantic role  →  component slot  →  product composition
                     canvas            surface           agent viewport
                     textSecondary     description       session metadata
                     primaryFill       button fill       Send / Confirm
                     danger            danger feedback   Discard confirmation
```

| Semantic role | Dark | Light | Midnight | Usage |
|---|---|---|---|---|
| `canvas` | `#0d1721` | `#f6f8fb` | `#080e16` | Primary workspace background |
| `chrome` | `#0d1720` | `#f4f7fb` | `#090f18` | 30px top + 26px bottom |
| `surface` | `#101b26` | `#ffffff` | `#0c1420` | Cards/dialogs |
| `surfaceDeep` | `#0c1620` | `#f8fafc` | `#090f18` | Side panels |
| `surfaceRaised` | `#14212d` | `#edf3f9` | `#121f30` | Hover, composer and popovers |
| `border` | `#263544` | `#d8e0e9` | `#233346` | Main separators |
| `text` | `#d8e0e9` | `#273544` | `#dbe6f6` | Main text |
| `textSecondary` | `#8b9aac` | `#5d6b7e` | `#95a8be` | Secondary text |
| `textTertiary` | `#8899aa` | `#607083` | `#8d9fb4` | Small readable metadata (proposed lift from V2 faint) |
| `success / danger / warning` | green / red / amber | darker variants | green / red / amber | State meaning only |

Accent supplies `accent`, `focusRing`, selected border/text, and `primaryFill` for actionable buttons. `focusRing` is **not** disabled for mouse use when an active keyboard focus is present. `primaryFill` is deliberately darker than V2's blue CTA to preserve white label contrast.

**Contrast:** body text 4.5:1 against its actual surface; important visual boundaries/icons 3:1; focus visible 3:1. Color cannot be sole indicator for diff lines, approval state, or warnings. Validate each new theme against actual component backgrounds (not just `canvas`).

## Type and hierarchy

| Slot | Desktop standard | Line height | Weight | Example |
|---|---|---|---|---|
| App chrome | 12.5–13px | 1.3–1.45 | 500 | Toolbar, session label |
| Body | 13px | 1.48 | 400 | Settings form and tree |
| Reading | 13–14px | 1.65 | 400 | Agent explanation |
| Metadata | 11.5–12px | 1.4 | 400 | Timestamp, duration |
| Code / Diff | 12px mono | 1.58 | 400 | Tool output and diff |
| Panel title | 16px | 1.3 | 620 | Changes heading |
| Page title | 18–22px | 1.3 | 620 | Settings page |

Fonts are preferences: UI Inter/system fallback, mono JetBrains Mono/Cascadia/Consolas fallback. Do not bundle copyrighted fonts without verified redistribution rights. Numbers in metrics and diff gutters use tabular numerals. Preserve Unicode/IME Vietnamese selection and composition.

## Spatial rhythm

Base spacing values: **2 / 4 / 6 / 8 / 12 / 16 / 20 / 24 / 32 / 40px**. Radius: **3 / 5 / 7 / 10 / 14px**. Use 1px hairlines; 2px focus and selected-left rails. Group by proximity: label/help 4, control groups 8–12, section groups 16–24, larger layouts 32.

Three densities:

| Size | Compact | Standard | Comfortable |
|---|---:|---:|---:|
| Explorer tree row | 23 | 27 | 32 |
| Repeated list row | 30 | 34 | 40 |
| Most form controls | 28 | 32 | 36 |
| Data table row | 26 | 30 | 36 |

Locked heights (all densities): top **30**, Agent buffer **34**, statusbar **26**. Interactive icons may be visually 15–17px, but have a >=28px desktop hit surface; strive for >=32 when space permits. All crucial icons require labels in tooltips/menus and keyboard equivalents.

## Desktop shell geometry

- Defaults at wide windows: Explorer **300px**, Changes **565px**, center flexes to remaining width.
- Explorer allowed 210–390px; Changes 340–720px; center minimum **420px at 100% UI scale**. These are proposals and should be tuned with actual text and font metrics.
- Narrow windows: under ~1140px auto-hide Changes if user has not explicitly pinned it; under ~850px also hide Explorer. Never make an invisible pane's command unreachable. Screen bounds are recomputed after UI scale or monitor changes.
- Panel resizing persists user override; handle is 1px visually, **8px hit region**. Double-click splitter can restore default. Avoid inadvertent drag from inline text selection.
- Conversation reading column max **790px**, centered in middle pane. Composer max roughly **826px**, fixed above status bar, with independent scroll for long content.
- Overlay ownership: app-level palette and confirmation modal may overlie panes; normal popovers anchor to owning control; terminal opens an actual bottom dock/panel without new redundant status controls.

## Motion and feedback

Use 0ms for immediate state, ~110ms hover/press, ~160ms disclosure, ~190ms overlay. No animated shimmer when data is already cached. On reduced-motion setting, transitions are 0ms, while status changes remain visible via icon/text. Status transitions respond to events, not hopeful timers.

## Icons

Prefer a single Lucide-like outline family (nominal 16px, 1.7–1.9px stroke) unless the current product has an established icon set. Match stroke weight and optical alignment; don't mix filled and outlined icons to represent equivalent actions. File type color is supplementary, not semantic status. Timestamps and counts should remain text, not icon-only.

## Default visual rules

One primary call to action per decision region; no cards inside cards except truly independent layered objects; section boundaries by space and hairlines; reserve saturated accents for focus, selected state and action. Do not rely on shadows as a substitute for hierarchy. If the UI looks coherent with accent colors turned off, hierarchy is doing its job.
