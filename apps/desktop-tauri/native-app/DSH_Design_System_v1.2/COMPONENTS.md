> **V1.2 source precedence:** The untouched approved `reference/dsh-ui-prototype-v2.html` owns visuals and `DSH_FEATURE_TRACEABILITY.md` plus live Host evidence own capability claims. This document is supplementary; it cannot override the frozen V2 layout or claim a feature exists.

# Component library — DSH UI contracts

Components implement a **behavior contract**, not merely a color/rectangle. Every interactive primitive exposes semantic label, enabled state, keyboard focus, hover/press/active, loading, error, and completion where applicable. GPUI code may use existing primitives or wrappers; do not force a new dependency before capability/version review.

## Required component API conventions

```text
Visual props:   variant, size, density?, emphasis?, label, icon?, description?
State:          disabled, loading, selected?, checked?, expanded?, invalid?
Events:         on_activate, on_change, on_dismiss, on_submit, on_cancel
Accessibility:  role, accessible_name, description, shortcut?, focus_target
Runtime:        pending_request_id?, outcome? (only when actually supported)
```

The names are a design-level interface; **not guaranteed actual Rust identifiers**. Domain components should keep typed state and callbacks (avoid opaque stringly-typed global event names). Non-interactive text needs no tabindex.

## A. Base primitives

| Component | Variants / slots | Required behavior | Keyboard & a11y |
|---|---|---|---|
| `DsButton` | primary, default, ghost, danger, icon; sm/md/lg | No double-submit while pending; loading preserves width; danger not conflated with warning | Enter/Space activates; visible focus; icon-only accessible name |
| `DsIconButton` | toolbar, close, disclosure, status | 28–32px hit box; tooltip action + shortcut; disabled state readable | Enter/Space; don't hide critical action behind hover |
| `DsTextField` | search, regular, inline; start/end icon | Explicit label and help, inline validation, clear button, committed vs draft value | Text editing, selection, IME; Enter behavior intentional |
| `DsTextArea` | plain, composer, log | Auto-grow up to max, then internal scroll; preserve selection; no text loss on error | Ctrl/Meta+Enter configurable, Shift+Enter newline |
| `DsSelect` | trigger + menu + search? | Distinguish selection from command; async options preserve focus | Arrows select, Enter commit, Esc cancel |
| `DsCheckbox` | normal, indeterminate | Independent on/off; pending writes show actual completion | Space toggles; label clickable |
| `DsSwitch` | immediately applied setting | Only for immediate toggle; rollback/report error if persistence fails | Space toggles, live checked state |
| `DsRadioGroup` | horizontal, vertical | Mutually exclusive small visible options; do not fake select list | Arrow navigation, one selected item |
| `DsSegmented` | 2–4 exclusive views | Stable selection; not for destructive commits | Arrows or Tab; announced state |
| `DsTooltip` | text + shortcut | Delay ~450ms; won't block keyboard; never sole instruction | Appears on keyboard focus; Esc dismiss |
| `DsBadge` | neutral, success, warning, danger, info | Only meaningful status/count, text always conveys meaning | Noninteractive |
| `DsDivider` | hairline, inset | Uses surface border, never used as primary layout strategy | Decorative |
| `DsProgress` | determinate, indeterminate | Determinate only with real total; remaining work not fabricated | Announces label/value; reduced-motion static alternative |
| `DsSkeleton` | text, row, card | Only while truly loading; replaces shape without layout jump | Hidden from reading order if decorative |
| `DsEmptyState` | title, cause, next action | Distinguish no content, no results, no permission and offline | Primary action reachable |
| `DsErrorState` | inline, region, full | Explain object, cause, actionable retry, retain user draft | Non-color cues and focus on error summary when appropriate |
| `DsToast` | info, success, warning, error | Event-driven; never sole feedback for critical failure; copyable error detail | Accessible status / alert; manual dismissal for critical |
| `DsInlineAlert` | info, warning, critical | Anchored near affected object, no layout thrash | Keyboard reach for action |
| `DsPopover` | anchored, menu-like, detail | Positioned within screen; closes Esc/outside; restores opener focus | Focus management, no accidental nested trap |
| `DsMenu` | action, checked, radio, submenu | Destructive actions separated and labeled; no ghost menu options | Up/Down, Enter, Esc, type-to-focus |
| `DsDialog` | confirm, form, approval | Title + object/scope + consequences + named primary action; clear Cancel | Focus trap only while modal; Esc safe; restore focus |
| `DsCommandPalette` | commands, files, sessions | Fuzzy search, recent, groups, keyboard hint, empty/loading; stable selection | Ctrl/Meta+Shift+P or existing binding; Arrow/Enter/Esc |
| `DsKbd` | chord, sequence | Displays platform-specific shortcuts, not a false keymap | Presentational |

## B. Navigation, data and surfaces

| Component | Parts | Contract |
|---|---|---|
| `DsTreeView` | row, twisty, depth guide, file icon, labels | Lazy children, preserved expansion by workspace, selected != hovered != focused, errors per node, virtualize when needed; left/right traversal. |
| `DsTabBar` | tab, active indicator, overflow menu | One active view, own pane context, track unsaved state if applicable; no duplicate Changes tab in center. |
| `DsListRow` | icon, title, subtitle, badge, trailing action | Fixed alignment lanes, truncation + full tooltip, stable selection identity. |
| `DsSearchResults` | section, result, match highlight, path/line | Distinguish content results vs commands, handle no results and cancellation; Enter navigates. |
| `DsResizablePane` | container, separator, collapse control | Mouse drag and keyboard resize where practical, clamp dimensions, persist overrides, reset default. |
| `DsPanelHeader` | title, count, toolbar | Count meaningful and real, actions in one predictable row. |
| `DsFilePath` | segments, truncate | Middle-truncate long paths; Copy full path; never hide filename extension as the only clue. |
| `DsCodeBlock` | language, copy, wrap toggle, selection | Monospace line fidelity. **Visual soft wrap must NOT insert hard newlines when copied.** Copy raw text, not screen-rendered rows. |
| `DsDiffViewer` | file header, hunk, gutter, +/- marks, code | Readable monospace; stage scope explicit (file vs hunk). Binary/no diff shown as factual explanation; conflict never presented as clean. |
| `DsStatusIndicator` | dot/icon + label/details | Connected/running/errors grounded in Host state; tooltip provides source and last refresh. Dot alone is insufficient in a critical workflow. |
| `DsResizableTerminal` | tablist, PTY viewport, connection state, pane actions | Rendering/wrapping vs copied payload separated; resize updates rows/cols; reconnect without assuming process exit. |

## C. Agent-specific product composites

| Component | Key slots / states | Definition of good UX |
|---|---|---|
| `SessionPicker` | current title, status, search, recent/pinned/archived, create | Bound to active workspace; changing session keeps draft and doesn't cancel run; no permanent sidebar session list. |
| `AgentTranscript` | messages, artifacts, run dividers, loading tail | Stable scroll anchor on streaming, explicit time role, code/markdown selectable, no invisible truncation. |
| `MessageBlock` | speaker, timestamp, body, attachments, retry? | User/assistant roles clear without heavy chat bubbles; selection/copy uninterrupted. |
| `ToolActivityGroup` | count, summary, expansion, status, duration | Condensed by default after completion; expand for inputs, stdout, stderr, exit code, with redaction policy; failure stays visible. |
| `ToolActivityRow` | icon, verb, target, duration, status | Pending/running/succeeded/failed/cancelled/unknown distinct; no automatic success upon UI event. |
| `ApprovalCard` | tool, exact target/scope, command preview, reason, approve-once, deny | **Host allows only allowed-once/rejected** in old map; never invent Always allow or expiry UI. Request idempotency and pending feedback. |
| `AgentComposer` | input, context mentions, attachments, model picker, Queue/Steer, send/stop | Draft per session, async model selection, send only on actual acceptance, attachment failures recoverable; never show invented Ask/Plan/Build mode. |
| `RunStateBanner` | state label, source, next action, follow-up | Differentiate running, waiting approval, detached, interrupted, disconnected, completed. No claims of autonomous background success without Host confirmation. |
| `ReviewActionBar` | file count, review, stage/unstage/discard, commit | Respect current selection/staged group; destructive scope written in confirmation; stale diff guard before mutation. |
| `GitOperationBanner` | operation, branch, conflicts, continue/abort | Don't offer continue before conditions; repo state from Host; resolve flow explicit. |
| `MarketplaceItem` | publisher, name, trust facts, permissions, install/enable/update | No unverified `Verified` badge. Install progress and rollback limitations explicit; permissions from actual source. |
| `PreferenceRow` | title, description, control, restart/apply effect | Clear local vs Host setting scope; conflict/restart behavior accurate; accessible control association. |

## D. State matrix template (mandatory for each interactive component)

| State | Visual | Interaction | A11y | Test |
|---|---|---|---|---|
| Rest | Normal foreground/surface | Action available | Name + role | Render default |
| Hover | Quiet surface difference | Pointer only | No semantic change | Hover test |
| Focus-visible | 2px high contrast ring | Keyboard still works | Focus announced | Keyboard capture |
| Pressed / open | Held state distinct from hover | Reversible until commit | Expanded/pressed state | Escape, focus restore |
| Disabled | Low emphasis but readable label | No action, no fake hover | Disabled declared | Handler not called |
| Loading | Button/field retains layout | Dedupe requests | Busy status if relevant | Double-click suppression |
| Error | Inline reason and recovery | User can retry/cancel | Error association | Injected failure |
| Success | UI reflects actual result | Further action coherent | Status when not obvious | Ack and no duplicate |

## E. Product components that should NOT be invented yet

`AlwaysAllowApproval`, `TrustedMarketplaceBadge`, `PlanModeSwitcher`, `ParallelRunScheduler`, `HardDeleteSession`, `AutomaticMergeResolver`: each needs its own Host/product contract. Render unsupported as honest explanatory affordance only if users need to know about it; otherwise omit.
