> **V1.2 source precedence:** The untouched approved `reference/dsh-ui-prototype-v2.html` owns visuals and `DSH_FEATURE_TRACEABILITY.md` plus live Host evidence own capability claims. This document is supplementary; it cannot override the frozen V2 layout or claim a feature exists.

# Interaction patterns — from components to trust

## 1. Action taxonomy

| Category | Example | Pattern |
|---|---|---|
| Navigation | Select file/session/tab | Immediate, reversible, no confirmation |
| Configuration | Change density/theme/keymap | Apply directly if local; persist async; revert + report error on failure |
| Runtime command | Send prompt, stage file, switch branch | Indicate pending, unique request ID when supported, bind outcome to Host ack |
| Destructive | Discard file, delete file, remove bundle | Show exact affected object + scope, affirmative destructive label, stale-state recheck |
| Permission | Execute shell / write outside granted scope | Host approval is authoritative; reason/scope, no implicit broad grants |

**Never** respond to button press by marking runtime success before RPC acknowledgement. Canceled or timed-out UI request does not necessarily mean the Host operation was canceled.

## 2. Overlay stack and focus

1. One global stack: tooltip → popover/menu → palette → dialog/approval (topmost captures input where warranted).
2. `Esc` dismisses topmost *dismissible* layer first; does not silently reject a Host approval unless the UX explicitly offers Cancel.
3. Opening records prior focus target; dismissal restores focus if the object still exists, else nearest stable region.
4. Arrow keys inside menu/list, Tab across controls, Enter invokes default, Space activates buttons/checkables. Never hijack text selection in text fields.
5. Menus within scrollable panes use overlay portal/window layer so pane clipping does not hide them; ensure accessible names in GPUI.

## 3. Streaming transcript and scroll

- If user is near bottom (e.g. within ~48px) when a new assistant delta appears, follow live output. If user has scrolled back, preserve anchor and show `N new updates` / `Jump to latest` instead of jerking viewport.
- Session switch saves each session's draft, scroll anchor and disclosure state locally. Returning reconstructs history from Host, not from stale rendered HTML.
- Tool output is collapsible with count + short summary. Failed tool remains visible, stderr is not lost; loading spinner is only for observed running state.
- Large output truncation must show **truncated** and the exact supported way to view more. Redact secrets before display/logging per existing Host policy, not ad-hoc substring replacements.
- Command and code copy copies the logical text. Line wrapping at terminal width must never become accidental newline characters in clipboard; preserve intentional newlines.

## 4. Context and composer

- Model picker exposes actual catalog/status (unavailable model explains why). Queue/Steer only if Host supports them; never synthesize a new execution mode.
- Attachments show filename, size, type, processing/progress/error/ready. Sending with unresolved attachments follows explicit policy; never silently drop a file.
- `@` mention resolves an actual workspace path and shows source-of-truth in a chip. Deleted/moved file yields stale context warning at send time.
- Disabled Send explains why (empty draft/blocked/no connection). Stop means cancel request and then reconcile with Host response; don't promise immediate process termination.
- If offline, preserve draft and offer Retry when connection recovers. Dedupe repeated submission after reconnect.

## 5. Review and Git

- Changes list: changed count, stage state, full path tooltip. Selecting a file must not mutate it. Use consistent abbreviations: M, A, D, R, U, each with text explanation.
- Diff: background tints supplement `+`/`-` and separate line numbers. Syntax colors must not reduce legibility. Long lines can horizontally scroll or show optional visual wrap without changing copy.
- Destructive flow: confirm file, path, type, current state and consequence. Re-fetch current status right before mutation if Host supports version guard; on conflict show `The file changed since you opened this diff`.
- Commit: live staged count, message required, submit pending, actual success from Git result. Never allow no-op-looking commit silently.
- Branch change with dirty working tree: show stale snapshot and original command errors, not a generic red toast.

## 6. Connectivity, failures and ambiguity

| Situation | UI language | Allowed actions |
|---|---|---|
| Host disconnected | `Cannot reach Host. A run may still be active.` | Reconnect, inspect cached history, keep draft |
| Run interrupted | `Run ended before completion.` + cause if known | Resume/retry according to runtime support |
| Approval pending | `Approval required to run ...` | Allow once / Deny, exact tool and target |
| Tool error | `Shell command failed (exit 1)` | Expand stderr, copy, retry if safe |
| Long task with hidden session | `Running in another session` only if supported | Navigate to session; do not imply future monitoring |
| No results | `No matching files` | Clear filter / search different scope |
| Missing capability | `Not supported by this Host version` | Safe alternate or no action; never fake done |

## 7. Accessibility / locale

- Aim for WCAG 2.2 AA applicable outcomes: minimum contrast, keyboard operability, visible focus, relationships, status announcements. Dense desktop isn't an excuse to omit focus or tooltips.
- Do not expose icon-only success/error. All form controls have name, current value, error text and cause. Modal title is read/announced before contents.
- Test Vietnamese Telex/VNI composition, combining characters, long file paths, CJK names, emoji and RTL text where supported by font stack. IME Enter must not submit mid-composition.
- Keyboard navigation is part of product quality, not an optional shortcut layer. Preserve OS text editing semantics.
- UI scale 100/125/150%: reflow, not clipping; long text truncates with tooltip, critical errors wrap. Reduced motion setting respected.

## 8. Perceived performance

- Respond to button activation immediately with a pressed/pending visual state; only show an indefinite spinner once there is an actual pending operation.
- Use optimistic UI only for reversibly local settings with rollback, NEVER for filesystem/approval/Git success.
- Virtualize large trees/lists while preserving focus/selection identity. Avoid rerendering entire transcript on each token if GPUI architecture allows incremental update.
- Separate frame/perceived latency measurement from Host duration and network IO. No invented SLA; baseline and profile on target hardware.
