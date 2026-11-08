> **V1.2 source precedence:** The untouched approved `reference/dsh-ui-prototype-v2.html` owns visuals and `DSH_FEATURE_TRACEABILITY.md` plus live Host evidence own capability claims. This document is supplementary; it cannot override the frozen V2 layout or claim a feature exists.

# Feature opportunity map — extend UI without turning DSH into an IDE

**Important:** These are UX proposals, NOT verified missing features. Agent D0 must check the currently running GPUI app and Host contracts, mark actual status, then rank by user pain. An interface feature needs a real data/action capability or an explicitly honest local-only implementation.

| ID | Candidate UX improvement | Why it matters | Rank | Capability / risk gate |
|---|---|---|---|---|
| O01 | First-use empty workspace/welcome | User understands next step before any session exists | P0 | Folder picker/workspace open; avoid fake sample project |
| O02 | Global run connection/blocked clarity | No guessing whether work stopped or only UI disconnected | P0 | session follow state; no false background claims |
| O03 | Unified command palette & discoverable shortcuts | Every important command reachable without pointer | P0 | existing action registry; one source of truth |
| O04 | Per-session draft and scroll restoration | Long running work remains navigable | P0 | local state + Host replay; handle late events |
| O05 | Copy-safe long code/command rendering | Prevent broken pasted commands and accidental newlines | P0 | logical text model, selection and clipboard tests |
| O06 | Tool output inspector with stderr/exit/duration | Debug failed agent steps without scanning logs | P0 | real Host event metadata or explicit unknown |
| O07 | Approval queue that names scope and source | Users know what they are allowing and why | P0 | Host approval request ID, only allowed-once/deny |
| O08 | Reliable file freshness/stale banners | Avoid editing deleted/renamed/modified files blindly | P0 | watchers, stale-version guard |
| O09 | Consistent loading/empty/error surfaces | Fewer ambiguous blank panes and fake spinners | P0 | can be client-only with real error payload |
| O10 | Inline keymap conflict diagnostics | Shortcut remaps become trustworthy and reversible | P1 | command registry + preference persistence |
| O11 | Session search in picker, with pinned/recent groups | Large history without permanent sidebar list | P1 | session search/list; stable IDs |
| O12 | Keyboard navigation across diff/tree/approval | Fast professional desktop operation | P1 | GPUI focus/action audit; tests |
| O13 | Native UX showcase / component gallery | Agent can inspect regressions faster | P1 | debug-only GPUI gallery; don't ship mock Host |
| O14 | Focus-mode toggle to hide both sides | Work comfortably in narrow windows | P1 | client-local layout, recovery shortcut |
| O15 | Clear changed-file grouping and review scope | Prevent accidental stage/discard of wrong data | P1 | Git status/diff and stale guards |
| O16 | Better marketplace permission/trust details | Understand risk before install | P1 | index trust facts, do not invent verified badges |
| O17 | Density/scale presets with live native preview | Different monitors and preferences | P1 | theme/local prefs; test clipped controls |
| O18 | Accessible status/error announcements | Help screen-reader/keyboard users understand changes | P1 | GPUI/accessibility support of pinned release |
| O19 | Session timeline search/filter | Find evidence in long task histories | P2 | Host search semantics, pagination, privacy |
| O20 | Layout reset and workspace override explanations | Recover from overly narrow/saved pane configurations | P2 | existing local prefs + migration policy |

## Ranking rule

Prioritize by `user frequency × severity × evidence × runtime feasibility`. Do not estimate with fake survey numbers. Audited `NOT_SUPPORTED` is a product decision, not permission to simulate an interaction.

## Out of scope until separately approved

Hard delete session, global always-allow permissions, tool-chain parallel scheduler, automatic Git conflict resolution, interactive rebase, extra IDE activity bar, unverified Marketplace `Verified` badges, runtime rewrite, and any GPUI major dependency switch.
